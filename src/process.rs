extern crate regex;
extern crate timer;
extern crate chrono;

use std::io;
use std::thread;
use std::process::{Command, Stdio, Child, ChildStdout};
use std::io::{BufReader, BufRead};
use std::sync::mpsc::channel;
use regex::Regex;

#[derive(Debug)]
pub enum Error {
    Process(io::Error),
    Tor(String, Vec<String>),
    InvalidLogLine,
    InvalidBootstrapLine(String),
    Regex(regex::Error),
    ProcessNotStarted,
    Timeout,
}

pub struct TorProcess {
    tor_cmd: String,
    args: Vec<String>,
    torrc_path: Option<String>,
    completion_percent: u8,
    timeout: u32,
    pub stdout: Option<BufReader<ChildStdout>>,
    pub process: Option<Child>,
}

impl TorProcess {
    pub fn new() -> Self {
        TorProcess {
            tor_cmd: "tor".to_string(),
            args: vec![],
            torrc_path: None,
            completion_percent: 100 as u8,
            timeout: 0 as u32,
            stdout: None,
            process: None,
        }
    }

    pub fn tor_cmd(&mut self, tor_cmd: &str) -> &mut Self {
        self.tor_cmd = tor_cmd.to_string();
        self
    }

    pub fn torrc_path(&mut self, torrc_path: &str) -> &mut Self {
        self.torrc_path = Some(torrc_path.to_string());
        self
    }

    pub fn arg(&mut self, arg: String) -> &mut Self {
        self.args.push(arg);
        self
    }

    pub fn args(&mut self, args: Vec<String>) -> &mut Self {
        for arg in args {
            self.arg(arg);
        }
        self
    }

    pub fn completion_percent(&mut self, completion_percent: u8) -> &mut Self {
        self.completion_percent = completion_percent;
        self
    }

    pub fn timeout(&mut self, timeout: u32) -> &mut Self {
        self.timeout = timeout;
        self
    }

    // The tor process will have its stdout piped, so if the stdout lines are not consumed they
    // will keep accumulating over time, increasing the consumed memory.
    pub fn launch(&mut self) -> Result<&mut Self, Error> {
        let mut tor = Command::new(&self.tor_cmd);
        if let Some(ref torrc_path) = self.torrc_path {
            tor.args(&vec!["-f", torrc_path]);
        }
        let mut tor_process = tor.args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| Error::Process(err))?;
        let stdout = BufReader::new(tor_process.stdout.take().unwrap());

        self.process = Some(tor_process);
        let completion_percent = self.completion_percent;

        let (stdout_tx, stdout_rx) = channel();
        let stdout_timeout_tx = stdout_tx.clone();

        let timer = timer::Timer::new();
        let _guard = timer.schedule_with_delay(chrono::Duration::seconds(self.timeout as i64),
                                               move || {
                                                   stdout_timeout_tx.send(Err(Error::Timeout))
                                                                    .unwrap_or(());
                                               });
        let stdout_thread = thread::spawn(move || {
            stdout_tx.send(Self::parse_tor_stdout(stdout, completion_percent)).unwrap_or(());
        });
        match stdout_rx.recv().unwrap() {
            Ok(stdout) => {
                stdout_thread.join().unwrap();
                self.stdout = Some(stdout);
                Ok(self)
            }
            Err(err) => {
                self.kill().unwrap_or(());
                stdout_thread.join().unwrap();
                Err(err)
            }
        }
    }

    fn parse_tor_stdout(mut stdout: BufReader<ChildStdout>,
                        completion_perc: u8)
                        -> Result<BufReader<ChildStdout>, Error> {
        let re_bootstrap = Regex::new(r"^\[notice\] Bootstrapped (?P<perc>[0-9]+)%: ")
            .map_err(|err| Error::Regex(err))?;

        let timestamp_len = "May 16 02:50:08.792".len();
        let mut warnings = Vec::new();
        let mut raw_line = String::new();

        while stdout.read_line(&mut raw_line).map_err(|err| Error::Process(err))? > 0 {
            {
                if raw_line.len() < timestamp_len + 1 {
                    return Err(Error::InvalidLogLine);
                }
                let timestamp = &raw_line[..timestamp_len];
                let line = &raw_line[timestamp_len + 1..raw_line.len() - 1];
                debug!("{} {}", timestamp, line);
                match line.split(' ').nth(0) {
                    Some("[notice]") => {
                        if let Some("Bootstrapped") = line.split(' ').nth(1) {
                            let perc = re_bootstrap.captures(line)
                                .and_then(|c| c.name("perc"))
                                .and_then( |pc| pc.as_str().parse::<u8>().ok())
                                .ok_or(Error::InvalidBootstrapLine(line.to_string()))?;

                            if perc >= completion_perc {
                                break;
                            }
                        }
                    }
                    Some("[warn]") => warnings.push(line.to_string()),
                    Some("[err]") => return Err(Error::Tor(line.to_string(), warnings)),
                    _ => (),
                }
            }
            raw_line.clear();
        }
        Ok(stdout)
    }

    pub fn kill(&mut self) -> Result<(), Error> {
        if let Some(ref mut process) = self.process {
            Ok(process.kill().map_err(|err| Error::Process(err))?)
        } else {
            Err(Error::ProcessNotStarted)
        }
    }
}

impl Drop for TorProcess {
    // kill the child
    fn drop(&mut self) {
        self.kill().unwrap_or(());
    }
}
