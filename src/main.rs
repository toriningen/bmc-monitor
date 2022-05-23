use walkdir::WalkDir;

use std::error::Error;
use std::thread::sleep;
use std::time::{Duration, Instant};
use std::path::{Path, PathBuf};
use std::iter::{Iterator, IntoIterator};
use std::fs;
use std::io;
use std::convert::{identity as id};
use std::process;

struct Fans {
    paths: Vec<PathBuf>
}

impl Fans {
    fn discover() -> Self {
        // iterate through sysfs and find all fans (fan*_input) that either have non-zero values or cannot be read due to errors
        // an example of fan path: /sys/devices/pci0000:00/0000:00:1f.3/i2c-0/0-002d/hwmon/hwmon3/fan1_input

        fn is_good_fan<P: AsRef<Path>>(path: P) -> Option<()> {
            let file_name: String = path.as_ref().file_name()?.to_str()?.into();

            if !file_name.starts_with("fan") {
                return None;
            }
            if !file_name.ends_with("_input") {
                return None;
            }

            Some(())
        }

        Self {
            paths: WalkDir::new("/sys")
                .into_iter()
                .filter_map(|ent| {
                    let ent = ent.ok()?;
                    let path = ent.into_path();

                    is_good_fan(&path).map(|()| path)
                })
                .collect()
        }
    }

    fn is_healthy(&self) -> bool {
        self.paths.iter().map(|path| {
            match fs::read_to_string(&path) {
                Ok(_) => true,
                Err(_) => false,
            }
        }).all(id)
    }
}

const RESTART_THRESHOLD: Duration = Duration::from_millis(5000);
const RECOVERY_THRESHOLD: Duration = Duration::from_millis(10000);

#[derive(Debug, Copy, Clone)]
enum FansState {
    Healthy,
    Failed {
        since: Instant,
    },
    Restarted {
        since: Instant,
    },
    FuckedUp,
}

fn main() -> Result<(), Box<dyn Error>> {
    use FansState::*;

    let fans = Fans::discover();
    let mut state: FansState = Healthy;

    loop {
        let is_healthy = fans.is_healthy();

        let next_state = if is_healthy {
            Healthy
        } else {
            let now = Instant::now();
            match state {
                Healthy => {
                    Failed { since: now }
                },
                Failed { since } => {
                    if since.elapsed() >= RESTART_THRESHOLD {
                        Restarted { since: now }
                    } else {
                        state
                    }
                },
                Restarted { since } => {
                    if since.elapsed() >= RECOVERY_THRESHOLD {
                        FuckedUp
                    } else {
                        state
                    }
                },
                FuckedUp => {
                    Restarted { since: now }
                }
            }
        };

        match (state, next_state) {
            (Failed { since }, Restarted { .. }) => {
                println!("Failed since {:?}, restarting BMC...", since);
                let sp: io::Result<bool> = (|| Ok(process::Command::new("ipmitool").args(["bmc", "reset", "cold"]).spawn()?.wait()?.success()) )();
                if let Err(err) = sp {
                    eprintln!("Couldn't reset BMC: {}", err);
                }
            },
            (Restarted { since }, FuckedUp) => {
                println!("Fucked up since {:?}, notifying about dead fans...", since);
                let sp: io::Result<bool> = (|| Ok(process::Command::new("notify_fan_failure.sh").spawn()?.wait()?.success()) )();
                if let Err(err) = sp {
                    eprintln!("Couldn't notify about fan failure: {}", err);
                }
            },
            _ => {},
        }

        //println!("state = {state:?}, next_state = {next_state:?}");

        state = next_state;

        sleep(Duration::from_millis(1000));
    }
}
