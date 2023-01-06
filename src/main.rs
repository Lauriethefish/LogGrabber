use std::{io::{self, BufWriter}, path::{Path, PathBuf}, time::Duration};

use adb::{Adb, DeviceStatus};
use zip::ZipWriter;

mod adb;

const LOGS_DIR: &str = "/sdcard/Android/data/com.beatgames.beatsaber/files/logs/";
const LOG_TO_SAVE: &str = "GlobalLog";
const TOMBSTONES_DIR: &str = "/sdcard/Android/data/com.beatgames.beatsaber/files/";
const SAVE_PATH: &str = "dump.zip";
const BMBF_CONF_PATH: &str = "/sdcard/BMBFData/config.json";

fn save_logcat(adb: &Adb, dump_dir: impl AsRef<Path>) -> Result<(), io::Error> {
    let mut out_file = BufWriter::new(std::fs::File::create(dump_dir.as_ref().join("adb.log"))?);
    adb.pipe_until_elapsed(&["logcat"], &mut out_file, Duration::from_secs(2))?;
    Ok(())
}

fn try_pull_file(adb: &Adb, from: &str, to: &str) {
    match adb.invoke_device_command(&["pull", &from, &to]) {
        Ok(_) => println!("Pulled {from} to {to}"),
        Err(err) => eprintln!("Failed to pull {from}: {err}")
    }
}

fn try_save_latest_with_prefix(adb: &Adb, from: &str, prefix: &str, to_dir: impl AsRef<Path>) {
    let files = match adb.list_files(from) {
        Ok(files) => files,
        Err(err) => {
            eprintln!("Failed to list logs directory: {err}");
            return;
        }
    };

    let (_time, file) = files.iter().fold((0, None), |(c_time, current), file| {
        if !file.starts_with(prefix) {
            return (c_time, current);
        }

        let path = PathBuf::from(from).join(&file).to_str().unwrap().to_string();
        let output = match adb.invoke_device_command(&["shell", "date", "+%s", "-r", &path]) {
            Ok(out) => out,
            Err(err) => { 
                eprintln!("Failed to date file: {path}: {err}");
                return(c_time, current);
            }
        };

        let time = String::from_utf8(output.stdout)
            .expect("Invalid UTF8 from `adb shell date`")
            .trim()
            .parse::<usize>()
            .expect("Invalid timestamp from `adb shell date`");

        if time > c_time {
            (time, Some(file))
        }   else    {
            (c_time, current)
        }
    });

    if let Some(file) = file {
        let from = PathBuf::from(from).join(file).to_str().unwrap().to_string();
        let to = to_dir.as_ref().join(file).to_str().unwrap().to_string();
    
        try_pull_file(adb, &from, &to);
    }
}

fn try_set_device(adb: &mut Adb, status: &DeviceStatus, id: &str, name: &str, unauth_text: &str) -> bool {
    match status {
        DeviceStatus::Ready => {
            println!("Chosen device: {id}");
            adb.set_device(id.to_owned());
            return true;
        },
        DeviceStatus::Unauthorized => println!("{name} is unauthorized. Please go into your headset and press `Allow` to give your computer access. {unauth_text}"),
        DeviceStatus::Other(status) => println!("{name} is reporting an unknown status: {status}. Please check your connection to your device and try another cable if necessary")
    }

    false
}

fn choose_device(adb: &mut Adb) -> Result<(), io::Error> {
    loop {
        let devices = adb.list_devices()?;
        if devices.len() == 1 {
            let (id, status) = devices.first().unwrap();
    
            if try_set_device(adb, status, id, "The connected device", "") {
                return Ok(());
            }

            println!("(Press enter to refresh)");
            let mut _data = String::new();
            std::io::stdin().read_line(&mut _data).unwrap();
        }   else    {
            println!("\nYou have multiple devices connected. Please enter the number of the one you would like to select: ");
            for (idx, (id, status)) in devices.iter().enumerate() {
                println!("{}) {id}: {status}", idx + 1);
            }

            let mut id = String::new();
            std::io::stdin().read_line(&mut id)?;

            match id.trim().parse::<usize>() {
                Ok(n) => if let Some((id, status)) = devices.get(n - 1) {
                    if try_set_device(adb, status, id, "The selected device", "Alternatively, select another device.") {
                        return Ok(());
                    }
                    continue;
                },
                Err(_) => {}
            };

            eprintln!("Invalid device ID")
        }

    }
}

fn setup_adb_and_temp_dir(f: impl FnOnce(PathBuf, &mut Adb)) {
    println!("Please wait while platform-tools extracts . . .");
    let global_temp_dir = std::env::temp_dir();

    for i in 0..5 {
        let temp_dir = if i == 0 {
            global_temp_dir.join("log-grabber")
        }   else    {
            global_temp_dir.join(format!("log-grabber-{i}"))
        };

        match std::fs::remove_dir_all(&temp_dir) {
            Ok(()) => {},
            Err(err) => { 
                if err.kind() != io::ErrorKind::NotFound {
                    eprintln!("Failed to remove existing temp folder: {err}");
                    eprintln!("Is another instance already running?");
                    eprintln!("Trying another temp folder");
                    continue;
                }
            }
        }

        let mut adb = match adb::Adb::extract_to(&temp_dir) {
            Ok(adb) => adb,
            Err(err) => {
                eprintln!("Failed to extract platform-tools: {err}");
                eprintln!("Is another instance already running?");
                return;
            }
        };

        (f)(temp_dir.clone(), &mut adb);


        match adb.invoke_command(&["kill-server"])
            .and_then(|_| std::fs::remove_dir_all(temp_dir)) {
            Ok(_) => {},
            Err(_) => eprintln!("Failed to delete temp dir on shutdown")
        }
        return;
    }

    eprintln!("Tried several times to remove temp folder without success")
}

fn zip_files(parent_folder: impl AsRef<Path>, output_path: impl AsRef<Path>) -> Result<(), std::io::Error> {
    let output_writer = std::fs::File::create(output_path)?;
    let mut archive = ZipWriter::new(output_writer);

    for file in std::fs::read_dir(parent_folder)? {
        let entry = file?;
        if entry.file_type()?.is_file() {
            let mut reader = std::fs::File::open(entry.path())?;
            archive.start_file(entry.file_name().to_str().unwrap(), zip::write::FileOptions::default())?;

            std::io::copy(&mut reader, &mut archive)?;
        }
    }

    archive.finish()?;
    Ok(())
}

fn dump(temp_dir: PathBuf, adb: &mut Adb) {
    match choose_device(adb) {
        Ok(()) => {},
        Err(err) => {
            eprintln!("Failed to set up device: {err}");
            return;
        }
    }

    println!("\nCreating dump: ");

    let dump_dir = temp_dir.join("dump");
    match std::fs::create_dir(&dump_dir) {
        Ok(_) => {},
        Err(err) => {
            println!("Failed to create dump directory: {err}");
            return;
        }
    }

    match save_logcat(&adb, &dump_dir) {
        Ok(_) => println!("Successfully saved logcat"),
        Err(err) => eprintln!("Failed to save logcat data: {err}"),
    }

    try_save_latest_with_prefix(adb, LOGS_DIR, LOG_TO_SAVE, &dump_dir);
    try_save_latest_with_prefix(adb, TOMBSTONES_DIR, "tombstone", &dump_dir);
    try_pull_file(adb, BMBF_CONF_PATH, dump_dir.join("bmbf_config.json").to_str().unwrap());

    // Now zip up the dump
    match zip_files(dump_dir, SAVE_PATH) {
        Ok(_) => println!("Successfully saved dump to {SAVE_PATH}"),
        Err(err) => eprintln!("Failed to save to {SAVE_PATH}: {err}"),
    }
}

fn main() {
    setup_adb_and_temp_dir(dump);
}