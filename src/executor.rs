struct Dispatcher {
	sep: std::sync::Mutex<(Executor, Spawner)>,
}

pub struct TimerFuture {
    shared_state: std::sync::Arc<std::sync::Mutex<SharedState>>,
}

/// Shared state between the future and the waiting thread
struct SharedState {
    /// Whether or not the sleep time has elapsed
    completed: bool,

    /// The waker for the task that `TimerFuture` is running on.
    /// The thread can use this after setting `completed = true` to tell
    /// `TimerFuture`'s task to wake up, see that `completed = true`, and
    /// move forward.
    waker: Option<std::task::Waker>,
}

impl futures::Future for TimerFuture {
    type Output = ();
    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        // Look at the shared state to see if the timer has already completed.
        let mut shared_state = self.shared_state.lock().unwrap();
        if shared_state.completed {
            std::task::Poll::Ready(())
        } else {
            // Set waker so that the thread can wake up the current task
            // when the timer has completed, ensuring that the future is polled
            // again and sees that `completed = true`.
            //
            // It's tempting to do this once rather than repeatedly cloning
            // the waker each time. However, the `TimerFuture` can move between
            // tasks on the executor, which could cause a stale waker pointing
            // to the wrong task, preventing `TimerFuture` from waking up
            // correctly.
            //
            // N.B. it's possible to check for this using the `Waker::will_wake`
            // function, but we omit that here to keep things simple.
            shared_state.waker = Some(cx.waker().clone());
            std::task::Poll::Pending
        }
    }
}

impl TimerFuture {
    /// Create a new `TimerFuture` which will complete after the provided
    /// timeout.
    pub fn new(duration: std::time::Duration) -> Self {
        let shared_state = std::sync::Arc::new(std::sync::Mutex::new(SharedState {
            completed: false,
            waker: None,
        }));

        // Spawn the new thread
        let thread_shared_state = shared_state.clone();

        std::thread::spawn(move || {
            std::thread::sleep(duration);
            let mut shared_state = thread_shared_state.lock().unwrap();
            // Signal that the timer has completed and wake up the last
            // task on which the future was polled, if one exists.
            shared_state.completed = true;
            if let Some(waker) = shared_state.waker.take() {
                waker.wake() // Get the executor to poll the future again.
            }
        });

        TimerFuture { shared_state }
    }
}

/// Task executor that receives tasks off of a channel and executes them.
pub struct Executor {
	/// Thread safe queue of finished tasks.
	ready_queue: std::sync::mpsc::Receiver<std::sync::Arc<Task>>,
}

/// Spawner spawns new futures onto the task channel.
#[derive(Clone)]
pub struct Spawner {
	task_sender: std::sync::mpsc::SyncSender<std::sync::Arc<Task>>,
}

/// A future that can reschedule itself to be polled by an executor.
struct Task {
	future: std::sync::Mutex<Option<futures::future::BoxFuture<'static, ()>>>,
	task_sender: std::sync::mpsc::SyncSender<std::sync::Arc<Task>>,
}

pub fn new_executor_and_spawner() -> (Executor, Spawner) {
	// Maximum number of tasks to allow queueing in the channel at once.
	const MAX_QUEUED_TASKS: usize = 10_000;

	let (task_sender, ready_queue) = std::sync::mpsc::sync_channel(MAX_QUEUED_TASKS);
	(Executor { ready_queue }, Spawner { task_sender })
}

use futures::FutureExt;
// The timer we wrote in the previous section:
//use timer_future::TimerFuture;

impl Spawner {
	/// Spawn a future onto the task channel.
	pub fn spawn(&self, future: impl std::future::Future<Output = ()> + Send + 'static) {
		let future = future.boxed();
		let task = std::sync::Arc::new(Task {
			future: std::sync::Mutex::new(Some(future)),
			task_sender: self.task_sender.clone(),
		});
		self.task_sender.send(task).expect("too many tasks queued");
	}
}

impl futures::task::ArcWake for Task {
	fn wake_by_ref(arc_self: &std::sync::Arc<Self>) {
		let cloned = arc_self.clone();
		arc_self.task_sender.send(cloned).expect("too many tasks queued");
	}
}

impl Executor {
	fn run(&self) {
		while let Ok(task) = self.ready_queue.recv_timeout(std::time::Duration::from_millis(16)) {
			let mut future_slot = task.future.lock().unwrap();
			if let Some(mut future) = future_slot.take() {
				let waker = futures::task::waker_ref(&task);
				let context = &mut futures::task::Context::from_waker(&*waker);
				if let std::task::Poll::Pending = future.as_mut().poll(context) {
					*future_slot = Some(future);
				}
			}
		}
	}
}

// #[cfg(test)]
// mod tests {
// 	use super::*;

// 	#[test]
// 	fn test() {
// 		let (executor, spawner) = new_executor_and_spawner();

// 		spawner.spawn(async {
// 			println!("howdy!");
// 			// Wait for our timer future to complete after two seconds.
// 			TimerFuture::new(std::time::Duration::new(2, 0)).await;
// 			println!("done!");
// 		});

// 		drop(spawner);

// 		executor.run();
// 	}

// 	// #[test]
// 	// fn test_systems_as_parameters() {
// 	// 	let mut dispatcher = Dispatcher::new();

// 	// 	let test_system = TestSystem::new();

// 	// 	let system_handle = dispatcher.add_system(test_system);

// 	// 	async fn task(orchestrator: &Orchestrator<'static>) {
// 	// 	 	// let test_system = orchestrator.get_system::<TestSystem>(system_handle).await;

// 	// 	 	// let test_system = test_system.lock().unwrap();

// 	// 	 	// assert_eq!(test_system.get_value(), 0);
// 	// 	};

// 	// 	dispatcher.execute_task_2(task);

// 	// 	dispatcher.update();
// 	// }
// }