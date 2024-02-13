use std::fs::File;
use std::io::{copy, Seek, SeekFrom, Write};


pub static mut LOG_FILE : Option<File> = None;

pub fn init_log() {
    let mut log_file_path = std::env::temp_dir();
    log_file_path.push("limiter.log");

    let mut file = File::options().write(true).read(true).create(true).open(log_file_path).unwrap();
    file.seek(SeekFrom::End(0)).unwrap(); //can't use append because we want to truncate during rotate_log
    unsafe { &mut LOG_FILE }.replace(file);
}

macro_rules! log {
    ($($args:tt)*) => {
        if let Some(ref mut file) = unsafe { &mut LOG_FILE } {
            use std::fmt::Write;
            let mut line = format!("[{}] ", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
            write!(line, $($args)*).unwrap();
            write!(line, "\n").unwrap();
            file.write(line.as_bytes()).unwrap();
        }
        else {
            panic!("No log file!");
        }
    }
}
pub(crate) use log;

pub fn flush_log() {
    if let Some(ref mut file) = unsafe { &mut LOG_FILE } {
        file.flush().unwrap();
    }
}

pub fn rotate_log() -> anyhow::Result<()> {
    let mut log_file = unsafe { LOG_FILE.as_mut().unwrap() };

    let mut historic_log_file_path = std::env::temp_dir();
    for i in 1..100 {
        historic_log_file_path.push(format!("limiter.{i}.log"));
        if !historic_log_file_path.exists() { break }
        historic_log_file_path.pop();
    }

    let mut historic_log_file = File::options().write(true).create(true).open(historic_log_file_path)?;

    log_file.rewind()?;
    copy(&mut log_file, &mut historic_log_file)?; //TODO(Rennorb) @perf: Copy whole file instead of file contents.

    if historic_log_file.flush().is_err() {
        log_file.seek(SeekFrom::End(0))?;
        return Ok(()); //TODO(Rennorb) @correctness: Which error to return here? ok is nor really correct.
    }

    log_file.set_len(0)?;
    log_file.rewind()?;

    Ok(())
}
