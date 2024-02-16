use chrono::{Datelike, Local, TimeZone};
use chrono::Timelike;
use std::ffi::OsString;
use std::fs::File;
use std::io::{Read, Write, Seek};
use std::str::FromStr;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::time::Duration;
use steam_api::structs::profile::{User, Users};
use windows_service::{define_windows_service, Result, service_control_handler, service_dispatcher};
use windows_service::service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType};
use windows_service::service_control_handler::{ServiceControlHandlerResult, ServiceStatusHandle};
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;
mod logging;
use logging::*;
use winrt_notification::{LoopableSound, Sound, Toast};

define_windows_service!(ffi_service_main, online_limiter_service_main);

const SERVICE_NAME: &str = "online-limiter";
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

fn main() -> Result<()> {
    init_log();
    let service_result = service_dispatcher::start(SERVICE_NAME, ffi_service_main);
    flush_log();
    service_result
}

fn online_limiter_service_main(arguments: Vec<OsString>) {
    if let Err(err) = register_event_handler(arguments) {
        log!("error, {err}");
    }
}

fn register_event_handler(_arguments: Vec<OsString>) -> anyhow::Result<()> {
    let (shutdown_tx, shutdown_rx) = mpsc::channel();

    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,

            ServiceControl::Stop => {
                shutdown_tx.send(()).unwrap();
                ServiceControlHandlerResult::NoError
            }

            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };
    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;

    let result = run_service(status_handle, shutdown_rx);

    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    log!("Service ended.");

    return result
}

fn run_service(status_handle: ServiceStatusHandle, shutdown_rx: Receiver<()>) -> anyhow::Result<()> {
    log!("Service started.");
    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let cur_ver = hkcu.open_subkey("ENVIRONMENT").unwrap();
    let api_key: String = match cur_ver.get_value("STEAM_API_KEY") {
        Ok(val) => val,
        Err(err) => {
            log!("Could not obtain API_KEY env var: {err}");
            return Ok(());
        }
    };
    let steam_id: String = match cur_ver.get_value("STEAM_ID") {
        Ok(val) => val,
        Err(err) => {
            log!("Could not obtain STEAM_ID env var: {err}");
            return Ok(());
        }
    };
    let ffox_file: String = match cur_ver.get_value("FFOX_RECOVERY_JSON_LOCATION") {
        Ok(val) => val,
        Err(err) => {
            log!("Could not obtain FFOX_RECOVERY_JSON_LOCATION env var: {err}");
            return Ok(());
        }
    };

    let steam_ids = vec![steam_id.as_str()];

    let allowed_duration = Duration::from_secs(4 * 60 * 60);
    let tick_interval = Duration::from_secs(30);
    assert_eq!(allowed_duration.as_secs() % tick_interval.as_secs(), 0); // needed for the is_zero check later on

    let mut storage_file_path = std::env::temp_dir();
    storage_file_path.push("countdown.txt");

    let mut storage_file = File::options().read(true).write(true).create(true).open(storage_file_path).unwrap();

    let mut duration_left;
    if storage_file.metadata().unwrap().len() > 0 {
        let last_modified = storage_file.metadata().unwrap().modified().unwrap();
        let now = Local::now();
        let last_midnight = Local.with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0).unwrap();
        if last_modified < std::time::SystemTime::from(last_midnight) {
            duration_left = allowed_duration;
            storage_file.set_len(0).unwrap();
            storage_file.rewind().unwrap();
            write!(storage_file, "{}", duration_left.as_secs()).unwrap();
            _ = rotate_log();
        } else {
            let mut dur_str = Default::default();
            _ = storage_file.read_to_string(&mut dur_str);
            duration_left = Duration::from_secs(u64::from_str(&dur_str).unwrap_or(allowed_duration.as_secs()));
        }
    } else {
        duration_left = allowed_duration;
    }

    loop {
        let old_duration_left = duration_left;

        let current_time = Local::now();
        if current_time.hour() == 0 && current_time.minute() == 0 {
            duration_left = allowed_duration;
            _ = rotate_log();
        }

        let sites = vec![
            "YouTube",
            "Twitch",
            "Disney+",
            "Netflix",
            "Prime Video"
        ];
        let tabs = fxtabs::open_tabs(ffox_file.as_str()).unwrap_or_else(|_err| Vec::new());
        let filtered_tabs: Vec<&str> = tabs.iter().filter(|tab| {
            for site in &sites {
                if tab.title.contains(site) {
                    return true;
                }
            }
            return false;
        }).map(|tab| { tab.title.as_str() }).collect();

        let users = steam_api::get_profile_info(&steam_ids, &api_key).unwrap_or_else(|err| {
            log!("error while reading steam api: {err}");
            Users::default()
        });
        let default_user = &User::default();
        let user = users.user.first().unwrap_or(default_user);
        let game_id = user.gameid.as_str();

        if (!game_id.is_empty() || !filtered_tabs.is_empty()) && !duration_left.is_zero() {
            duration_left -= tick_interval;
        }

        if old_duration_left != duration_left {
            storage_file.set_len(0).unwrap();
            storage_file.rewind().unwrap();
            write!(storage_file, "{}", duration_left.as_secs()).unwrap();
        }

        if duration_left.is_zero() {
            no_gaming();
        }

        let joined_tab_string = filtered_tabs.join(", ");
        log!("Found Tabs: {joined_tab_string}");
        log!("Current gameid: {game_id}");
        log!("Time left: {duration_left:?}");
        if duration_left == tick_interval * 10 { //TODO(Markus) @cleanup: cleanup when moving tick_interval/max_duration into environment variables
            if let Err(err) = Toast::new(Toast::POWERSHELL_APP_ID).title("Online-Limiter")
                .text1(format!("Only {duration_left:?} left.").as_str())
                .sound(Some(Sound::Loop(LoopableSound::Alarm4)))
                .duration(winrt_notification::Duration::Long)
                .show()
            {
                log!("Error while sending notification: {err}");
                return Ok(());
            }
        }
        match shutdown_rx.recv_timeout(tick_interval) {
            // Break the loop either upon stop or channel disconnect
            Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => break,

            // Continue work if no events were received within the timeout
            Err(mpsc::RecvTimeoutError::Timeout) => (),
        };
    }
    Ok(())
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
                .args(["/C", std::format!("taskkill /F /IM {process_name}").as_str()]).output();
        }
    }
}
