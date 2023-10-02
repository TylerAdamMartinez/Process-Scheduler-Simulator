use nix::unistd::Pid;
use std::error::Error;
use std::ffi::OsStr;
use std::process::Command;
use std::sync::mpsc;
use std::time::SystemTime;
use ulid::Ulid;

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum Status {
    Running,
    Terminated(ExitCode),
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum ExitCode {
    Success,
    Failure,
}

impl std::fmt::Display for ExitCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExitCode::Success => write!(f, "0 (SUCCESS)"),
            ExitCode::Failure => write!(f, "1 (FAILURE)"),
        }
    }
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum State {
    New,
    Ready,
    Running,
    Waiting,
    Terminated,
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            State::New => write!(f, "NEW"),
            State::Ready => write!(f, "READY"),
            State::Running => write!(f, "RUNNING"),
            State::Waiting => write!(f, "WAITING"),
            State::Terminated => write!(f, "TERMINATED"),
        }
    }
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum Space {
    User,
    Kernal,
}

pub struct Task<'a> {
    pub state: State,
    pub duration: f64,
    pub priority: u8,
    pub exit_code: Option<ExitCode>,

    id: Ulid,
    pid: Option<Pid>,
    path_to_binary: &'a OsStr,
    args: Option<Vec<&'a str>>,
    created: SystemTime,
    space: Space,
}

impl<'a> Task<'a> {
    pub fn new(
        path_to_binary: &'a OsStr,
        args: Option<Vec<&'a str>>,
        space: Space,
        priority: u8,
    ) -> Self {
        Self {
            id: Ulid::new(),
            pid: None,
            path_to_binary,
            args,
            duration: 0.0,
            state: State::New,
            priority,
            space,
            exit_code: None,
            created: SystemTime::now(),
        }
    }

    pub fn get_id(&self) -> Ulid {
        self.id
    }

    pub fn get_space(&self) -> Space {
        self.space
    }

    pub fn get_date_time_created(&self) -> SystemTime {
        self.created
    }

    pub fn run(&mut self, tx: mpsc::Sender<Status>) {
        if self.pid.is_none() {
            self.state = State::Running;

            let mut command = Command::new(&self.path_to_binary);

            if let Some(arguments) = &self.args {
                command.args(arguments);
            }

            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(err) => {
                    self.exit_code = Some(ExitCode::Failure);
                    self.state = State::Terminated;
                    let now = SystemTime::now();
                    self.duration += now.duration_since(self.created).unwrap().as_secs_f64();

                    self.print_with_error(&err);

                    tx.send(Status::Terminated(ExitCode::Failure)).unwrap();
                    return;
                }
            };

            self.pid = Some(Pid::from_raw(child.id() as i32));

            match child.try_wait() {
                Ok(Some(exit_status)) => {
                    if exit_status.success() {
                        self.exit_code = Some(ExitCode::Success);
                        tx.send(Status::Terminated(ExitCode::Success)).unwrap();
                    } else {
                        self.exit_code = Some(ExitCode::Failure);
                        tx.send(Status::Terminated(ExitCode::Failure)).unwrap();
                    }

                    self.state = State::Terminated;
                    let now = SystemTime::now();
                    self.duration += now.duration_since(self.created).unwrap().as_secs_f64();
                    self.print();
                    return;
                }
                Ok(None) => {
                    self.state = State::Running;
                    self.print();
                    tx.send(Status::Running).unwrap();
                    self.pause();
                    return;
                }
                Err(err) => {
                    let now = SystemTime::now();
                    self.duration += now.duration_since(self.created).unwrap().as_secs_f64();
                    self.exit_code = Some(ExitCode::Failure);
                    self.state = State::Terminated;
                    self.print_with_error(&err);

                    tx.send(Status::Terminated(ExitCode::Failure)).unwrap();
                    return;
                }
            }
        } else {
            self.resume();
        }
    }

    pub fn pause(&mut self) {
        if let Some(pid) = self.pid {
            nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGSTOP).unwrap();

            self.state = State::Waiting;
            println!(
                "------------------------------------------\n\
                 PAUSED\n\
                 PID:            {}\n\
                 State:          {}\n\
                 ------------------------------------------",
                self.id, self.state,
            );
        }
    }

    pub fn resume(&mut self) {
        if let Some(pid) = self.pid {
            nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGCONT).unwrap();

            self.state = State::Running;
            println!(
                "------------------------------------------\n\
                 RESUMED\n\
                 PID:            {}\n\
                 State:          {}\n\
                 ------------------------------------------",
                self.id, self.state,
            );
        }
    }

    pub fn print(&self) {
        if self.state == State::Ready
            || self.state == State::Running
            || self.state == State::Waiting
        {
            println!(
                "------------------------------------------\n\
                 PID:            {}\n\
                 State:          {}\n\
                 ------------------------------------------",
                self.id, self.state,
            );
            return;
        }

        let exit_code_str = self
            .exit_code
            .as_ref()
            .map_or("-".to_string(), |e| e.to_string());

        println!(
            "------------------------------------------\n\
             PID:            {}\n\
             State:          {}\n\
             Exit Code:      {}\n\
             Duration:       {} seconds\n\
             ------------------------------------------",
            self.id, self.state, exit_code_str, self.duration,
        );
    }

    pub fn print_with_error(&self, err: &dyn Error) {
        let exit_code_str = self
            .exit_code
            .as_ref()
            .map_or("-".to_string(), |e| e.to_string());

        println!(
            "------------------------------------------\n\
             PID:            {}\n\
             State:          {}\n\
             Exit Code:      {}\n\
             Error Message:  {}\n\
             ------------------------------------------",
            self.id, self.state, exit_code_str, err
        );
    }

    pub fn get_current_state(&self) -> Result<Status, nix::errno::Errno> {
        if let Some(pid) = self.pid {
            match nix::sys::wait::waitpid(pid, Some(nix::sys::wait::WaitPidFlag::WNOHANG)) {
                Ok(nix::sys::wait::WaitStatus::StillAlive) => Ok(Status::Running),
                Ok(nix::sys::wait::WaitStatus::Exited(_, exit_code)) => {
                    if exit_code == 0 {
                        return Ok(Status::Terminated(ExitCode::Success));
                    } else {
                        return Ok(Status::Terminated(ExitCode::Failure));
                    }
                }
                Ok(_) => Ok(Status::Terminated(ExitCode::Failure)),
                Err(err) => Err(err),
            }
        } else {
            return Err(nix::errno::Errno::ESRCH);
        }
    }
}
