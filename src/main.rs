use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime};
use task::Task;
mod task;

const TIME_QUANTUM: u64 = 150;

fn dispatcher(tasks: &mut Vec<Task>, tx: &mpsc::Sender<task::ExitStatus>) {
    for task in tasks.iter_mut() {
        if task.state == task::State::Waiting {
            task.state = task::State::Ready;
        }
    }

    if let Some(task) = tasks
        .iter_mut()
        .filter(|t| t.state == task::State::Ready)
        .min_by_key(|t| t.priority)
    {
        println!(
            "Dispatcher selected PID: {} with priority: {}",
            task.get_id(),
            task.priority
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
        Task::new("/bin/ls".as_ref(), None, task::Space::Kernal, 3),
        Task::new(
            "/bin/cat".as_ref(),
            Some(Vec::from(["src/main.rs"])),
            task::Space::User,
            1,
        ),
        Task::new(
            "/bin/echo".as_ref(),
            Some(Vec::from(["Howdy Y'all!"])),
            task::Space::User,
            2,
        ),
        Task::new("/bin/ls".as_ref(), None, task::Space::Kernal, 5),
        Task::new("/bad/path/to/ls".as_ref(), None, task::Space::User, 4),
    ];

    for task in &mut tasks {
        println!(
            "Created PID: {} with priority: {} in space: {:?}",
            task.get_id(),
            task.priority,
            task.get_space(),
        );
        task.state = task::State::Ready;
    }

    loop {
        let mut all_done = true;

        dispatcher(&mut tasks, &tx);
        thread::sleep(Duration::from_millis(TIME_QUANTUM));

        for task in &mut tasks {
            if task.state == task::State::Running {
                match task.get_current_state() {
                    Ok(task::ExitStatus::Running) => {
                        task.pause();
                    }
                    Ok(task::ExitStatus::Terminated(task::ExitCode::Success)) => {
                        task.exit_code = Some(task::ExitCode::Success);
                        let now = SystemTime::now();
                        task.duration += now
                            .duration_since(task.get_date_time_created())
                            .unwrap()
                            .as_secs_f64();
                        task.state = task::State::Terminated;
                        task.print();
                    }
                    Ok(task::ExitStatus::Terminated(task::ExitCode::Failure)) => {
                        task.exit_code = Some(task::ExitCode::Failure);
                        let now = SystemTime::now();
                        task.duration += now
                            .duration_since(task.get_date_time_created())
                            .unwrap()
                            .as_secs_f64();
                        task.state = task::State::Terminated;
                        task.print();
                    }
                    Err(err) => {
                        let now = SystemTime::now();
                        task.duration += now
                            .duration_since(task.get_date_time_created())
                            .unwrap()
                            .as_secs_f64();
                        task.state = task::State::Terminated;
                        task.print_with_error(&err);
                    }
                }
            }

            if task.state != task::State::Terminated {
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
