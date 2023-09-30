use std::error::Error;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use ulid::Ulid;

const TIME_QUANTUM: u64 = 500;

#[derive(Debug, PartialEq, Copy, Clone)]
enum ExitStatus {
    Pending,
    Terminated(ExitCode),
}

#[derive(Debug, PartialEq, Copy, Clone)]
enum ExitCode {
    Success,
    Failure,
    From(i32),
}

impl std::fmt::Display for ExitCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExitCode::Success => write!(f, "0"),
            ExitCode::Failure => write!(f, "1"),
            ExitCode::From(code) => write!(f, "{}", code),
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
    path_to_binary: String,
    args: Option<Vec<String>>,
    duration: u64,
    state: ProcessState,
    priority: u8,
    space: ProcessSpace,
    exit_code: Option<ExitCode>,
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
            path_to_binary: path_to_binary.to_owned(),
            args,
            duration: 0,
            state: ProcessState::New,
            priority,
            space,
            exit_code: None,
        }
    }

    fn run(&mut self, quantum: u64, tx: mpsc::Sender<ExitStatus>) {
        self.state = ProcessState::Running;
        print_state(self.id, &self.state, self.exit_code, self.duration, None);

        let run_time = std::cmp::min(self.duration, quantum);
        self.duration -= run_time;

        let start_time = Instant::now();

        let mut command = Command::new(&self.path_to_binary);

        if let Some(arguments) = &self.args {
            command.args(arguments);
        }

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(err) => {
                let elapsed = start_time.elapsed();
                self.duration += elapsed.as_millis() as u64;
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
        };
        let exit_status = match child.wait() {
            Ok(exit_status) => exit_status,
            Err(err) => {
                let elapsed = start_time.elapsed();
                self.duration += elapsed.as_millis() as u64;
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
        };

        let elapsed = start_time.elapsed();
        self.duration += elapsed.as_millis() as u64;

        tx.send(ExitStatus::Terminated(ExitCode::Success)).unwrap();

        if exit_status.success() {
            self.exit_code = Some(ExitCode::Success);
            self.state = ProcessState::Terminated;
            print_state(self.id, &self.state, self.exit_code, self.duration, None);
        } else {
            self.state = ProcessState::Waiting;
            print_state(self.id, &self.state, self.exit_code, self.duration, None);
        }
    }
}

fn print_state(
    id: Ulid,
    state: &ProcessState,
    exit_code: Option<ExitCode>,
    duration: u64,
    err: Option<&dyn Error>,
) {
    if *state == ProcessState::Ready || *state == ProcessState::Running {
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
             Duration:       {}ms\n\
             ------------------------------------------",
            id, state, exit_code_str, duration,
        );
    };
}
fn dispatcher(tasks: &mut Vec<Task>, tx: &mpsc::Sender<ExitStatus>) {
    for task in &mut *tasks {
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
        task.run(TIME_QUANTUM, mpsc::Sender::clone(tx));
    }
}

fn main() {
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

        for task in &tasks {
            if task.state != ProcessState::Terminated {
                print_state(task.id, &task.state, task.exit_code, task.duration, None);
                all_done = false;
            }
        }

        if all_done {
            break;
        }

        dispatcher(&mut tasks, &tx);
        thread::sleep(Duration::from_millis(TIME_QUANTUM));
    }

    for _ in 0..tasks.len() {
        let _ = rx.recv().unwrap();
    }

    println!("All tasks completed!");
}
