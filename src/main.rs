use chrono::{Datelike, Local, TimeZone};
use chrono::Timelike;
use std::io::{Read, Write, Seek};
use std::str::FromStr;
use std::time::Duration;
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

fn main() -> anyhow::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let cur_ver = hkcu.open_subkey("ENVIRONMENT").unwrap();
    let api_key: String = match cur_ver.get_value("STEAM_API_KEY") {
        Ok(val) => val,
        Err(err) => {
            println!("Could not obtain API_KEY env var: {err}");
            return Ok(());
        }
    };
    let steam_id: String = match cur_ver.get_value("STEAM_ID") {
        Ok(val) => val,
        Err(err) => {
            println!("Could not obtain STEAM_ID env var: {err}");
            return Ok(());
        }
    };

    let steam_ids = vec![steam_id.as_str()];

    let allowed_duration = Duration::from_secs(4 * 60 * 60);
    let tick_interval = Duration::from_secs(30);
    assert_eq!(allowed_duration.as_secs() % tick_interval.as_secs(), 0); // needed for the is_zero check later on

    let mut storage_file_path = std::env::temp_dir();
    storage_file_path.push("countdown.txt");

    let mut storage_file = std::fs::File::options().read(true).write(true).create(true).open(storage_file_path).unwrap();

    let mut duration_left;
    if storage_file.metadata().unwrap().len() > 0 {
        let last_modified = storage_file.metadata().unwrap().modified().unwrap();
        let now = Local::now();
        let last_midnight = Local.with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0).unwrap();
        if last_modified < std::time::SystemTime::from(last_midnight) {
            duration_left = allowed_duration;
            storage_file.rewind().unwrap();
            write!(storage_file, "{}", duration_left.as_secs()).unwrap();
        } else {
            let mut dur_str = Default::default();
            _ = storage_file.read_to_string(&mut dur_str);
            duration_left = Duration::from_secs(u64::from_str(&dur_str).unwrap_or(allowed_duration.as_secs()));
        }
    } else {
        duration_left = allowed_duration;
    }

    loop {
        for user in steam_api::get_profile_info(&steam_ids, &api_key)?.user {
            let old_duration_left = duration_left;

            let current_time = chrono::prelude::Local::now();
            if current_time.hour() == 0 && current_time.minute() == 0 {
                duration_left = allowed_duration;
            }
            let game_id = user.gameid;
            if !game_id.is_empty() && !duration_left.is_zero() {
                duration_left -= tick_interval;
            }

            if old_duration_left != duration_left {
                storage_file.rewind().unwrap();
                write!(storage_file, "{}", duration_left.as_secs()).unwrap();
            }

            if duration_left.is_zero() {
                no_gaming();
            }
            println!("Time left: {duration_left:?}");
            println!("Current gameid: {game_id}");
            std::thread::sleep(tick_interval);
        }
    }
}

fn no_gaming() {
    let games = vec![
        ("1086940", vec!["bg3.exe", "bg3_dx11.exe"]), //Baldur's Gate 3
        ("671860", vec!["BattleBit.exe"]), //BattleBit Remastered
        ("227300", vec!["eurotrucks2.exe"]) //Euro Truck Simulator 2
    ];

    for (_process_id, process_names) in games {
        for process_name in process_names {
            let _ = std::process::Command::new("cmd")
                .args(["/C", std::format!("taskkill /IM {process_name}").as_str()]).output();
        }
    }
}