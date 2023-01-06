use std::{path::{Path, PathBuf}, io::{self, Read, Seek}, process::{Command, Stdio, Output}, time::{Duration, Instant}, fmt::Display};

use zip::result::ZipError;

#[cfg(target_os = "windows")]
const PLATFORM_TOOLS_ARCHIVE: &[u8] = include_bytes!("../plat_tools_win.zip");

#[cfg(target_os = "macos")]
const PLATFORM_TOOLS_ARCHIVE: &[u8] = include_bytes!("../plat_tools_mac.zip");

#[cfg(target_os = "linux")]
const PLATFORM_TOOLS_ARCHIVE: &[u8] = include_bytes!("../plat_tools_linux.zip");

#[cfg(target_os = "windows")]
const ADB_PATH_SUFFIX: &str = "platform-tools/adb.exe";

#[cfg(not(target_os = "windows"))]
const ADB_PATH_SUFFIX: &str = "platform-tools/adb.exe";

struct SliceReader<'a> {
    slice: &'a [u8],
    pos: usize
}

impl Read for SliceReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let bytes_left = self.slice.len() - self.pos;
        let bytes_to_read = std::cmp::min(buf.len(), bytes_left);

        buf.copy_from_slice(&self.slice[self.pos..self.pos + bytes_to_read]);
        self.pos += bytes_to_read;

        Ok(bytes_to_read)
    }
}

#[derive(Clone, Debug)]
pub enum DeviceStatus {
    Ready,
    Unauthorized,
    Other(String)
}

impl Display for DeviceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            DeviceStatus::Ready => "Ready to use",
            DeviceStatus::Unauthorized => "Unauthorized",
            DeviceStatus::Other(status) => &status
        })
    }
}

impl Seek for SliceReader<'_> {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        match pos {
            io::SeekFrom::Start(offset) => self.pos = offset as usize,
            io::SeekFrom::End(offset) => self.pos = (self.slice.len() as i64 + offset) as usize,
            io::SeekFrom::Current(offset) => self.pos = (self.pos as i64 + offset) as usize,
        }

        Ok(self.pos as u64)
    }
}

pub struct Adb {
    exe_loc: PathBuf,
    device_args: Option<Vec<String>>
}

impl Adb {
    pub fn extract_to(path: impl AsRef<Path>) -> Result<Adb, ZipError> {
        let mut plat_tools = zip::ZipArchive::new(SliceReader { 
            slice: PLATFORM_TOOLS_ARCHIVE,
            pos: 0
        }).unwrap();

        plat_tools.extract(&path)?;

        Ok(Self {
            exe_loc: path.as_ref().join(ADB_PATH_SUFFIX),
            device_args: None
        })
    }

    pub fn list_files(&self, dir: &str) -> Result<Vec<String>, io::Error> {
        let output = self.invoke_device_command(&["shell", "ls", dir])?;
        let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8 returned from `adb shell ls`");

        Ok(stdout.split("\n").map(|s| s.trim().to_owned()).collect())
    }

    fn device_command(&self, args: &[&str]) -> Command {
        let mut comm = Command::new(&self.exe_loc);
        comm.args(self.device_args.as_ref().expect("No device set").iter().map(|s| s.as_str()).chain(args.into_iter().map(|s| *s)));
        comm
    }

    pub fn list_devices(&self) -> Result<Vec<(String, DeviceStatus)>, io::Error> {
        let output = self.invoke_command(&["devices"])?;
        let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8 returned from `adb devices`");

        let mut devices = Vec::new();
        for line in stdout.lines().skip(1) {
            if line.trim().is_empty() {
                continue;
            }

            let mut iter = line.split_whitespace();
            let id = iter.next().expect("Invalid `adb devices` output");
            let status_str = iter.next().expect("Invalid `adb devices` output");

            let status = if status_str == "device" {
                DeviceStatus::Ready
            }  else if status_str == "unauthorized" {
                DeviceStatus::Unauthorized
            }   else    {
                DeviceStatus::Other(status_str.to_owned())
            };

            devices.push((id.to_owned(), status));
        }

        Ok(devices)
    }

    pub fn set_device(&mut self, device_id: String) {
        self.device_args = Some(vec!["-s".to_owned(), device_id]);
    }

    pub fn invoke_device_command(&self, args: &[&str]) -> Result<Output, io::Error> {
        self.device_command(args).output()
    }

    pub fn invoke_command(&self, args: &[&str]) -> Result<Output, io::Error> {
        Command::new(&self.exe_loc)
            .args(args)
            .output()
    }

    pub fn pipe_until_elapsed<'a>(&self,
        args: &[&str],
        out_stream: &mut impl io::Write,
        timeout: Duration) -> Result<(), io::Error> {
        let mut child = self.device_command(args)
            .stdout(Stdio::piped())
            .spawn()?;
        let spawn_time = Instant::now();

        let mut stdout = child.stdout.take().unwrap();

        let thread = std::thread::spawn(move || {
            loop {
                // If the ADB process has already exited, we can stop here
                if let Ok(Some(_)) = child.try_wait() {
                    return;
                }

                // Check if enough time has elapsed to kill the process prematurely
                let elapsed = spawn_time.elapsed();
                if elapsed > timeout {
                    break;
                }

                std::thread::sleep(Duration::from_millis(500));
            }

            child.kill().expect("Failed to kill child process; this should not happen");
        });

        std::io::copy(&mut stdout, out_stream)?;
        thread.join().unwrap();

        Ok(())
    }
}