use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use ulid::Ulid;

const TIME_QUANTUM: u64 = 500; 

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
    duration: u64,
    state: ProcessState,
    priority: u8,
    space: ProcessSpace,
    aged: bool,
}

impl Task {
    fn new(duration: u64, space: ProcessSpace, priority: u8) -> Self {
        Self {
            id: Ulid::new(),
            duration,
            state: ProcessState::New,
            priority,
            space,
            aged: false,
        }
    }

    fn run(&mut self, quantum: u64, tx: mpsc::Sender<usize>) {
        self.state = ProcessState::Running;
        println!("PID: {} transitioned to {:?}", self.id, self.state);

        let run_time = std::cmp::min(self.duration, quantum);
        self.duration -= run_time;

        thread::spawn(move || {
            thread::sleep(Duration::from_millis(run_time));
            tx.send(run_time as usize).unwrap();
        });
        
        if self.duration == 0 {
            self.state = ProcessState::Terminated;
            println!("PID: {} transitioned to {:?}", self.id, self.state);
        } else {
            self.state = ProcessState::Waiting;
            self.aged = true;
            println!("PID: {} transitioned to {:?}", self.id, self.state);
        }
    }
}

fn dispatcher(tasks: &mut Vec<Task>, tx: &mpsc::Sender<usize>) {
    for task in &mut *tasks {
        if task.aged && task.state == ProcessState::Waiting {
            task.state = ProcessState::Ready;
        }
    }

    if let Some(task) = tasks.iter_mut().filter(|t| t.state == ProcessState::Ready).min_by_key(|t| t.priority) {
        println!("Dispatcher selected PID: {} with priority: {}", task.id, task.priority);
        task.run(TIME_QUANTUM, mpsc::Sender::clone(tx));
    }
}

fn main() {
    let (tx, rx) = mpsc::channel();

    let mut tasks = vec![
        Task::new(100, ProcessSpace::Kernal, 3),
        Task::new(600, ProcessSpace::User, 1),
        Task::new(300, ProcessSpace::User, 2),
        Task::new(800, ProcessSpace::Kernal, 5),
        Task::new(50, ProcessSpace::User, 4),
    ];
   
    for task in &mut tasks {
        println!("Created PID: {} with priority: {}, duration: {}ms, in space: {:?}", task.id, task.priority, task.duration, task.space);
        task.state = ProcessState::Ready;
        println!("PID: {} transitioned to {:?}", task.id, task.state);
    }

    loop {
        let mut all_done = true;

        for task in &tasks {
            if task.state != ProcessState::Terminated {
                println!("PID: {} is currently in state: {:?}", task.id, task.state);
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
