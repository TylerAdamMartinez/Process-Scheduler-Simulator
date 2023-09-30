use nix::unistd::Pid;
use std::error::Error;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime};
use ulid::Ulid;

const TIME_QUANTUM: u64 = 150;

#[derive(Debug, PartialEq, Copy, Clone)]
enum ExitStatus {
    Running,
    Terminated(ExitCode),
}

#[derive(Debug, PartialEq, Copy, Clone)]
enum ExitCode {
    Success,
    Failure,
}

impl std::fmt::Display for ExitCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExitCode::Success => write!(f, "0"),
            ExitCode::Failure => write!(f, "1"),
        }
    }
}

#[derive(Debug, PartialEq)]
enum ProcessState {
    New,
    Ready,
    Running,
    Waiting,
    Terminated,
}

#[derive(Debug, PartialEq)]
enum ProcessSpace {
    User,
    Kernal,
}

struct Task {
    id: Ulid,
    pid: Option<Pid>,
    path_to_binary: String,
    args: Option<Vec<String>>,
    duration: f64,
    state: ProcessState,
    priority: u8,
    space: ProcessSpace,
    exit_code: Option<ExitCode>,
    created: SystemTime,
}

impl Task {
    fn new(
        path_to_binary: &str,
        args: Option<Vec<String>>,
        space: ProcessSpace,
        priority: u8,
    ) -> Self {
        Self {
            id: Ulid::new(),
            pid: None,
            path_to_binary: path_to_binary.to_owned(),
            args,
            duration: 0.0,
            state: ProcessState::New,
            priority,
            space,
            exit_code: None,
            created: SystemTime::now(),
        }
    }

    fn run(&mut self, tx: mpsc::Sender<ExitStatus>) {
        if self.pid.is_none() {
            self.state = ProcessState::Running;

            let mut command = Command::new(&self.path_to_binary);

            if let Some(arguments) = &self.args {
                command.args(arguments);
            }

            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(err) => {
                    self.exit_code = Some(ExitCode::Failure);
                    self.state = ProcessState::Terminated;
                    let now = SystemTime::now();
                    self.duration += now.duration_since(self.created).unwrap().as_secs_f64();

                    print_state(
                        self.id,
                        &self.state,
                        Some(ExitCode::Failure),
                        self.duration,
                        Some(&err),
                    );

                    tx.send(ExitStatus::Terminated(ExitCode::Failure)).unwrap();
                    return;
                }
            };

            self.pid = Some(Pid::from_raw(child.id() as i32));

            match child.try_wait() {
                Ok(Some(exit_status)) => {
                    if exit_status.success() {
                        self.exit_code = Some(ExitCode::Success);
                        tx.send(ExitStatus::Terminated(ExitCode::Success)).unwrap();
                    } else {
                        self.exit_code = Some(ExitCode::Failure);
                        tx.send(ExitStatus::Terminated(ExitCode::Failure)).unwrap();
                    }

                    self.state = ProcessState::Terminated;
                    let now = SystemTime::now();
                    self.duration += now.duration_since(self.created).unwrap().as_secs_f64();
                    print_state(self.id, &self.state, self.exit_code, self.duration, None);
                    return;
                }
                Ok(None) => {
                    self.state = ProcessState::Running;
                    print_state(self.id, &self.state, None, self.duration, None);
                    tx.send(ExitStatus::Running).unwrap();
                    self.pause();
                    return;
                }
                Err(err) => {
                    let now = SystemTime::now();
                    self.duration += now.duration_since(self.created).unwrap().as_secs_f64();
                    self.exit_code = Some(ExitCode::Failure);
                    self.state = ProcessState::Terminated;
                    print_state(
                        self.id,
                        &self.state,
                        self.exit_code,
                        self.duration,
                        Some(&err),
                    );

                    tx.send(ExitStatus::Terminated(ExitCode::Failure)).unwrap();
                    return;
                }
            }
        } else {
            self.resume();
        }
    }

    fn pause(&mut self) {
        if let Some(pid) = self.pid {
            nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGSTOP).unwrap();

            self.state = ProcessState::Waiting;
            println!(
                "------------------------------------------\n\
                 PAUSED\n\
                 PID:            {}\n\
                 State:          {:?}\n\
                 ------------------------------------------",
                self.id, self.state,
            );
        }
    }

    fn resume(&mut self) {
        if let Some(pid) = self.pid {
            nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGCONT).unwrap();

            self.state = ProcessState::Running;
            println!(
                "------------------------------------------\n\
                 RESUMED\n\
                 PID:            {}\n\
                 State:          {:?}\n\
                 ------------------------------------------",
                self.id, self.state,
            );
        }
    }
}

fn print_state(
    id: Ulid,
    state: &ProcessState,
    exit_code: Option<ExitCode>,
    duration: f64,
    err: Option<&dyn Error>,
) {
    if *state == ProcessState::Ready
        || *state == ProcessState::Running
        || *state == ProcessState::Waiting
    {
        println!(
            "------------------------------------------\n\
             PID:            {}\n\
             State:          {:?}\n\
             ------------------------------------------",
            id, state,
        );
        return;
    }

    let exit_code_str = exit_code
        .as_ref()
        .map_or("-".to_string(), |e| e.to_string());

    if let Some(ref err) = err {
        println!(
            "------------------------------------------\n\
             PID:            {}\n\
             State:          {:?}\n\
             Exit Code:      {}\n\
             Error Message:  {}\n\
             ------------------------------------------",
            id, state, exit_code_str, err
        );
    } else {
        println!(
            "------------------------------------------\n\
             PID:            {}\n\
             State:          {:?}\n\
             Exit Code:      {}\n\
             Duration:       {} seconds\n\
             ------------------------------------------",
            id, state, exit_code_str, duration,
        );
    };
}

fn dispatcher(tasks: &mut Vec<Task>, tx: &mpsc::Sender<ExitStatus>) {
    for task in tasks.iter_mut() {
        if task.state == ProcessState::Waiting {
            task.state = ProcessState::Ready;
        }
    }

    if let Some(task) = tasks
        .iter_mut()
        .filter(|t| t.state == ProcessState::Ready)
        .min_by_key(|t| t.priority)
    {
        println!(
            "Dispatcher selected PID: {} with priority: {}",
            task.id, task.priority
        );
        task.run(mpsc::Sender::clone(tx));
    }
}

fn main() {
    if !cfg!(unix) {
        println!("This program only runs on UNIX-like operating systems.");
        std::process::exit(1);
    }

    let (tx, rx) = mpsc::channel();

    let mut tasks = vec![
        Task::new("/bin/ls", None, ProcessSpace::Kernal, 3),
        Task::new(
            "/bin/cat",
            Some(Vec::from([String::from("src/main.rs")])),
            ProcessSpace::User,
            1,
        ),
        Task::new(
            "/bin/echo",
            Some(Vec::from([String::from("Howdy Y'all!")])),
            ProcessSpace::User,
            2,
        ),
        Task::new("/bin/ls", None, ProcessSpace::Kernal, 5),
        Task::new("/bad/path/to/ls", None, ProcessSpace::User, 4),
    ];

    for task in &mut tasks {
        println!(
            "Created PID: {} with priority: {} in space: {:?}",
            task.id, task.priority, task.space
        );
        task.state = ProcessState::Ready;
    }

    loop {
        let mut all_done = true;

        dispatcher(&mut tasks, &tx);
        thread::sleep(Duration::from_millis(TIME_QUANTUM));

        for task in &mut tasks {
            if task.state == ProcessState::Running {
                if let Some(pid) = task.pid {
                    match nix::sys::wait::waitpid(pid, Some(nix::sys::wait::WaitPidFlag::WNOHANG)) {
                        Ok(nix::sys::wait::WaitStatus::StillAlive) => {
                            task.pause();
                        }
                        Ok(nix::sys::wait::WaitStatus::Exited(_pid, exit_code)) => {
                            if exit_code == 0 {
                                task.exit_code = Some(ExitCode::Success);
                            } else {
                                task.exit_code = Some(ExitCode::Failure);
                            }

                            let now = SystemTime::now();
                            task.duration +=
                                now.duration_since(task.created).unwrap().as_secs_f64();
                            task.state = ProcessState::Terminated;
                            print_state(task.id, &task.state, task.exit_code, task.duration, None);
                        }
                        Ok(_) => {
                            let now = SystemTime::now();
                            task.duration +=
                                now.duration_since(task.created).unwrap().as_secs_f64();
                            task.state = ProcessState::Terminated;
                            print_state(task.id, &task.state, None, task.duration, None);
                        }
                        Err(err) => {
                            let now = SystemTime::now();
                            task.duration +=
                                now.duration_since(task.created).unwrap().as_secs_f64();
                            task.state = ProcessState::Terminated;
                            print_state(task.id, &task.state, None, task.duration, Some(&err));
                        }
                    }
                }
            }

            if task.state != ProcessState::Terminated {
                all_done = false;
            }
        }

        if all_done {
            break;
        }
    }

    for _ in 0..tasks.len() {
        let _ = rx.recv().unwrap();
    }

    println!("All tasks completed!");
}
