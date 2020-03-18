//! # Manager
//!
//! `taskmanager` contains the implementation of TaskManager

//
//   Shell-Core
//   Developed by Christian Visintin
//
// MIT License
// Copyright (c) 2020 Christian Visintin
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.
//

use super::{Task, TaskError, TaskErrorCode, TaskManager, TaskMessageRx, TaskMessageTx, TaskRelation};

use std::sync::{Arc, mpsc, Mutex};
use std::thread;
use std::time::Duration;

impl TaskManager {

    /// ### new
    /// 
    /// Instantiate a new TaskManager
    /// The first task in the task pipeline has to be provided
    pub(crate) fn new(first_task: Task) -> TaskManager {
        TaskManager {
            running: Arc::new(Mutex::new(false)),
            joined: Arc::new(Mutex::new(false)),
            m_loop: None,
            receiver: None,
            sender: None,
            next: Some(first_task)
        }
    }

    /// ### start
    /// 
    /// Start the task manager
    pub fn start(&mut self) -> Result<(), TaskError> {
        //Check if already running
        let mut running = self.running.lock().unwrap();
        if *running {
            return Err(TaskError::new(TaskErrorCode::AlreadyRunning, String::from("Could not start the TaskManager since it is already running")))
        }
        //If next is None, task manager has already terminated
        if self.next.is_none() {
            return Err(TaskError::new(TaskErrorCode::ProcessTerminated, String::from("TaskManager has already terminated")))
        }
        //Create channels
        let (rx_sender, rx_receiver) = mpsc::channel();
        let (tx_sender, tx_receiver) = mpsc::channel();
        self.receiver = Some(rx_receiver);
        self.sender = Some(tx_sender);
        let running_rc = Arc::clone(&self.running);
        let joined_rc = Arc::clone(&self.joined);
        //Get process out from TaskManager
        let task = self.next.take().unwrap();
        //Set running to true
        *running = true;
        //Start thread
        self.m_loop = Some(thread::spawn(move || {
            TaskManager::run(task, tx_receiver, rx_sender, running_rc, joined_rc)
        }));
        Ok(())
    }

    /// ### fetch_message
    /// 
    /// Returns TaskMessage in the RX inbox
    pub fn fetch_messages(&self) -> Result<Vec<TaskMessageRx>, TaskError> {
        let mut inbox: Vec<TaskMessageRx> = Vec::new();
        let receiver: &mpsc::Receiver<TaskMessageRx> = match &self.receiver {
            Some(recv) => recv,
            None => return Err(TaskError::new(TaskErrorCode::ProcessTerminated, String::from("It was not possible to collect messages, since the task is not running")))
        };
        loop {
            match receiver.try_recv() {
                Ok(message) => inbox.push(message),
                Err(err) => match err {
                    mpsc::TryRecvError::Empty => break,
                    _ => return Err(TaskError::new(TaskErrorCode::ProcessTerminated, String::from("It was not possible to collect messages, since the task is not running")))
                }
            }
        }
        Ok(inbox)
    }

    /// ### send_message
    /// 
    /// Send a message to the TaskManager thread
    pub fn send_message(&self, message: TaskMessageTx) -> Result<(), TaskError> {
        match &self.sender {
            None => Err(TaskError::new(TaskErrorCode::ProcessTerminated, String::from("TaskManager is not running"))),
            Some(sender) => {
                match sender.send(message) {
                    Ok(()) => Ok(()),
                    Err(_) => Err(TaskError::new(TaskErrorCode::ProcessTerminated, String::from("Receiver hung up")))
                }
            }
        }
    }

    /// ### is_running
    /// 
    /// Returns whether the TaskManager thread is still running or not
    pub fn is_running(&self) -> bool {
        let running = self.running.lock().unwrap();
        *running
    }

    /// ### join
    /// 
    /// Join Task Manager
    /// NOTE: this function is blocking, use is_running to join asynchronously
    pub fn join(&mut self) -> Result<u8, TaskError> {
        if self.m_loop.is_some() {
            //Set join to true
            {
                let mut joined = self.joined.lock().unwrap();
                *joined = true;
            }
            let rc: u8 = self.m_loop.take().map(thread::JoinHandle::join).unwrap().unwrap();
            //Set to none all the structures
            self.receiver = None;
            self.sender = None;
            Ok(rc)
        } else {
            Err(TaskError::new(TaskErrorCode::ProcessTerminated, String::from("Could not join task manager, since process has never been started")))
        }
    }

    /// ### run
    /// 
    /// Run method for thread
    fn run(mut task: Task, tx_receiver: mpsc::Receiver<TaskMessageTx>, rx_sender: mpsc::Sender<TaskMessageRx>, running: Arc<Mutex<bool>>, joined: Arc<Mutex<bool>>) -> u8 {
        //Start process
        if let Err(err) = task.start() {
            //Report error
            if rx_sender.send(TaskMessageRx::Error(err)).is_err() {
                //Set running to false
                TaskManager::false_running(running);
                return 255 //The other end hung up, so  terminate the thread
            }
        }
        let mut last_exit_code: u8 = 255;
        let mut terminate_called: bool = false;
        //Iterate over all tasks
        loop {
            if terminate_called {
                last_exit_code = 130;
                break;
            }
            //Always try to read before handling process running state
            match task.read() {
                Ok((stdout, stderr)) => {
                    //Send stdout and stderr (only if at least one of them is Some)
                    if stdout.is_some() || stderr.is_some() {
                        if rx_sender.send(TaskMessageRx::Output((stdout, stderr))).is_err() {
                            //Set running to false
                            TaskManager::false_running(running);
                            return 255 //The other end hung up, so  terminate the thread
                        }
                    }
                },
                Err(err) => {
                    match err.code {
                        TaskErrorCode::ProcessTerminated => {}, //If process terminated, ignore the error
                        _ => { //Otherwise report it
                            if rx_sender.send(TaskMessageRx::Error(err)).is_err() {
                                //Set running to false
                                TaskManager::false_running(running);
                                return 255 //The other end hung up, so  terminate the thread
                            }
                        }
                    }
                }
            }
            //If process is running handle Inputs
            if task.is_running() {
                //Handle TX receiver
                loop { //Iterate until all messages have been processes
                    match tx_receiver.try_recv() {
                        Ok(message) => {
                            match message { //Match received message type
                                TaskMessageTx::Input(input) => {
                                    //Try Write to process
                                    if let Err(err) = task.write(input) {
                                        //Report error in writing to process' stdin
                                        if rx_sender.send(TaskMessageRx::Error(err)).is_err() {
                                            //Set running to false
                                            TaskManager::false_running(running);
                                            return 255 //The other end hung up, so  terminate the thread
                                        }
                                    }
                                },
                                TaskMessageTx::Kill => {
                                    //Try Kill process
                                    if let Err(err) = task.kill() {
                                        //Report error in killing process
                                        if rx_sender.send(TaskMessageRx::Error(err)).is_err() {
                                            //Set running to false
                                            TaskManager::false_running(running);
                                            return 255 //The other end hung up, so  terminate the thread
                                        }
                                    }
                                },
                                TaskMessageTx::Signal(signal) => {
                                    //Try Send signal to process
                                    if let Err(err) = task.raise(signal) {
                                        //Report error in signaling process
                                        if rx_sender.send(TaskMessageRx::Error(err)).is_err() {
                                            //Set running to false
                                            TaskManager::false_running(running);
                                            return 255 //The other end hung up, so  terminate the thread
                                        }
                                    }
                                },
                                TaskMessageTx::Terminate => {
                                    //Kill task
                                    let _ = task.kill();
                                    //Set terminate called to true
                                    terminate_called = true;
                                    //Break
                                    break;
                                }
                            }
                        },
                        Err(recv_error) => {
                            match recv_error {
                                mpsc::TryRecvError::Empty => break, //No more messages, break from loop
                                _ => {
                                    //Kill process and return, the endpoint hung up
                                    let _ = task.kill();
                                    //Set running to false
                                    TaskManager::false_running(running);
                                    return 255
                                }
                            }
                        }
                    }
                }
                //If process is running, sleep for 100ms
                thread::sleep(Duration::from_millis(100));
            } else { //@! Otherwise handle next process in pipeline
                //@! Start next process or break from loop if pipeline has terminated
                //The next process is always pushed as new process. It may not be started though
                //In case the process is not INTENTIONALLY started (for example because the expression failed)
                // The next process will be executed if exists on the next cycle
                //Set last exitcode to task exitcode if it has Some
                last_exit_code = task.get_exitcode().unwrap_or(last_exit_code);
                match task.next {
                    Some(t) => {
                        //Start next task based on relation
                        match task.relation {
                            TaskRelation::And => { //Start new process ONLY if this process' exitcode was SUCCESSFUL
                                task = *t;
                                if last_exit_code == 0 {
                                    //Start next process
                                    if let Err(err) = task.start() {
                                        //Report error in starting process
                                        if rx_sender.send(TaskMessageRx::Error(err)).is_err() {
                                            //Set running to false
                                            TaskManager::false_running(running);
                                            return 255 //The other end hung up, so  terminate the thread
                                        }
                                    }
                                }
                            },
                            TaskRelation::Or => { //Start new process ONLY if this process' exitcode was UNSUCCESSFUL
                                task = *t;
                                if last_exit_code != 0 { //@! OR relation was UNSUCCESSFUL
                                    //Start next process
                                    if let Err(err) = task.start() {
                                        //Report error in starting process
                                        if rx_sender.send(TaskMessageRx::Error(err)).is_err() {
                                            //Set running to false
                                            TaskManager::false_running(running);
                                            return 255 //The other end hung up, so  terminate the thread
                                        }
                                    }
                                } else { //@! OR relation was successful
                                    //Else set next process as next->next until out of OR relation; 
                                    //NOTE: this will be handled by the next iteration though
                                    continue;
                                }
                            },
                            TaskRelation::Pipe => {
                                //Set task to next and nothing else (NOTE: the process is already running)
                                task = *t;
                            },
                            TaskRelation::Unrelated => {
                                //Just start next process
                                task = *t;
                                if let Err(err) = task.start() {
                                    //Report error in starting process
                                    if rx_sender.send(TaskMessageRx::Error(err)).is_err() {
                                        //Set running to false
                                        TaskManager::false_running(running);
                                        return 255 //The other end hung up, so  terminate the thread
                                    }
                                }
                            }
                        }
                    },
                    None => break //No remaining process in pipeline, terminate
                }
            }
        }
        //Set running to false
        TaskManager::false_running(running);
        //Wait for join to be called
        loop {
            {
                let joined = joined.lock().unwrap();
                if *joined {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
        //Return exitcode
        task.exit_code.unwrap_or(last_exit_code)
    }

    /// ### false_running
    /// 
    /// Set running to false
    fn false_running(running: Arc<Mutex<bool>>) {
        let mut running = running.lock().unwrap();
        *running = false;
    }

}

//@! Module Test

#[cfg(test)]
mod tests {

    use super::*;
    use crate::Redirection;
    use crate::UnixSignal;

    use std::thread::sleep;
    use std::time::{Duration, Instant};

    #[test]
    fn test_manager_new() {
        let command: Vec<String> = vec![String::from("echo"), String::from("foobar")];
        let sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Verify manager
        assert!(!manager.is_running());
        assert!(manager.m_loop.is_none());
        assert!(manager.next.is_some());
        assert!(manager.receiver.is_none());
        assert!(manager.sender.is_none());
        //Join must fail
        assert_eq!(manager.join().err().unwrap().code, TaskErrorCode::ProcessTerminated);
        //Send must fail
        assert_eq!(manager.send_message(TaskMessageTx::Kill).err().unwrap().code, TaskErrorCode::ProcessTerminated);
        //Receive must fail
        assert_eq!(manager.fetch_messages().err().unwrap().code, TaskErrorCode::ProcessTerminated);
    }

    #[test]
    fn test_manager_one_task() {
        let command: Vec<String> = vec![String::from("echo"), String::from("foobar")];
        let sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Start
        assert!(manager.start().is_ok());
        //Wait for receiveing message
        let start_time: Instant = Instant::now(); //Timeout for 3 seconds
        let mut message_recv: bool = false;
        while !message_recv {
            if start_time.elapsed().as_secs() >= 3 {
                panic!("test_manager_one_task : TaskManager timeout");
            }
            //Get message
            let inbox: Vec<TaskMessageRx> = manager.fetch_messages().unwrap();
            for message in inbox.iter() {
                match message {
                    TaskMessageRx::Output((stdout, _stderr)) => {
                        assert_eq!(*stdout.as_ref().unwrap(), String::from("foobar\n"));
                        println!("test_manager_one_task : Received message from task: '{}'", stdout.as_ref().unwrap());
                        message_recv = true;
                    },
                    TaskMessageRx::Error(err) => panic!("test_manager_one_task : Unexpected error: {:?}", err)
                }
            }
            sleep(Duration::from_millis(100));
        }
        //Verify exit code
        assert!(!manager.is_running());
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 0);
    }

    #[test]
    fn test_manager_pipeline_unrelated() {
        //Build pipeline
        let command: Vec<String> = vec![String::from("echo"), String::from("foo")];
        let mut sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let command: Vec<String> = vec![String::from("echo"), String::from("bar")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Unrelated,
        );
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Start
        assert!(manager.start().is_ok());
        //Wait for output messages
        let start_time: Instant = Instant::now(); //Timeout for 3 seconds
        //We expect 2 Output messages
        let mut output_messages: u8 = 0;
        while output_messages < 2 {
            if start_time.elapsed().as_secs() >= 3 {
                panic!("test_manager_pipeline_unrelated : TaskManager timeout");
            }
            //Get message
            let inbox: Vec<TaskMessageRx> = manager.fetch_messages().unwrap();
            for message in inbox.iter() {
                match message {
                    TaskMessageRx::Output((stdout, _stderr)) => {
                        output_messages += 1;
                        match output_messages {
                            1 => assert_eq!(*stdout.as_ref().unwrap(), String::from("foo\n")),
                            2 => assert_eq!(*stdout.as_ref().unwrap(), String::from("bar\n")),
                            _ => panic!("Received a 3rd message from task... That was unexpected...")
                        }
                        println!("test_manager_pipeline_unrelated : Received message from task: '{}'", stdout.as_ref().unwrap());
                    },
                    TaskMessageRx::Error(err) => panic!("test_manager_pipeline_unrelated : Unexpected error: {:?}", err)
                }
            }
            sleep(Duration::from_millis(100));
        }
        //Verify exit code
        assert!(!manager.is_running());
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 0);
    }

    #[test]
    fn test_manager_pipeline_and_successful() {
        //Build pipeline
        let command: Vec<String> = vec![String::from("echo"), String::from("foo")];
        let mut sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let command: Vec<String> = vec![String::from("echo"), String::from("bar")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::And,
        );
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Start
        assert!(manager.start().is_ok());
        //Wait for output messages
        let start_time: Instant = Instant::now(); //Timeout for 3 seconds
        //We expect 2 Output messages
        let mut output_messages: u8 = 0;
        while output_messages < 2 {
            if start_time.elapsed().as_secs() >= 3 {
                panic!("test_manager_pipeline_and_successful : TaskManager timeout");
            }
            //Get message
            let inbox: Vec<TaskMessageRx> = manager.fetch_messages().unwrap();
            for message in inbox.iter() {
                match message {
                    TaskMessageRx::Output((stdout, _stderr)) => {
                        output_messages += 1;
                        match output_messages {
                            1 => assert_eq!(*stdout.as_ref().unwrap(), String::from("foo\n")),
                            2 => assert_eq!(*stdout.as_ref().unwrap(), String::from("bar\n")),
                            _ => panic!("Received a 3rd message from task... That was unexpected...")
                        }
                        println!("test_manager_pipeline_and_successful : Received message from task: '{}'", stdout.as_ref().unwrap());
                    },
                    TaskMessageRx::Error(err) => panic!("test_manager_pipeline_and_successful : Unexpected error: {:?}", err)
                }
            }
            sleep(Duration::from_millis(100));
        }
        //Verify exit code
        assert!(!manager.is_running());
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 0);
    }

    #[test]
    fn test_manager_pipeline_and_unsuccessful() {
        //Build pipeline
        let command: Vec<String> = vec![String::from("echo"), String::from("foo")];
        let mut sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let command: Vec<String> = vec![String::from("echo"), String::from("bar")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::And,
        );
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Start
        assert!(manager.start().is_ok());
        //Wait for output messages
        let start_time: Instant = Instant::now(); //Timeout for 3 seconds
        //We expect 2 Output messages
        let mut output_messages: u8 = 0;
        while output_messages < 2 {
            if start_time.elapsed().as_secs() >= 3 {
                panic!("test_manager_pipeline_and_unsuccessful : TaskManager timeout");
            }
            //Get message
            let inbox: Vec<TaskMessageRx> = manager.fetch_messages().unwrap();
            for message in inbox.iter() {
                match message {
                    TaskMessageRx::Output((stdout, _stderr)) => {
                        output_messages += 1;
                        match output_messages {
                            1 => assert_eq!(*stdout.as_ref().unwrap(), String::from("foo\n")),
                            2 => assert_eq!(*stdout.as_ref().unwrap(), String::from("bar\n")),
                            _ => panic!("Received a 3rd message from task... That was unexpected...")
                        }
                        println!("test_manager_pipeline_and_unsuccessful : Received message from task: '{}'", stdout.as_ref().unwrap());
                    },
                    TaskMessageRx::Error(err) => panic!("test_manager_pipeline_and_unsuccessful : Unexpected error: {:?}", err)
                }
            }
            sleep(Duration::from_millis(100));
        }
        //Verify exit code
        assert!(!manager.is_running());
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 0);
    }

    #[test]
    fn test_manager_pipeline_or_successful() {
        //Build pipeline
        let command: Vec<String> = vec![String::from("echo"), String::from("foo")];
        let mut sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let command: Vec<String> = vec![String::from("echo"), String::from("bar")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Or,
        );
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Start
        assert!(manager.start().is_ok());
        //Wait for output messages
        let start_time: Instant = Instant::now(); //Timeout for 1 seconds
        //We expect 1 Output messages. Since the first process is successful, the second process should not be executed
        let mut output_messages: u8 = 0;
        loop {
            if start_time.elapsed().as_secs() >= 3 {
                panic!("test_manager_pipeline_unrelated : TaskManager timeout");
            }
            if !manager.is_running() {
                break;
            }
            //Get message
            let inbox: Vec<TaskMessageRx> = manager.fetch_messages().unwrap();
            for message in inbox.iter() {
                match message {
                    TaskMessageRx::Output((stdout, _stderr)) => {
                        output_messages += 1;
                        match output_messages {
                            1 => assert_eq!(*stdout.as_ref().unwrap(), String::from("foo\n")),
                            2 => panic!("test_manager_pipeline_or_successful : expected one output, but got 2"),
                            _ => panic!("Received a 3rd message from task... That was unexpected...")
                        }
                        println!("test_manager_pipeline_unrelated : Received message from task: '{}'", stdout.as_ref().unwrap());
                    },
                    TaskMessageRx::Error(err) => panic!("test_manager_pipeline_unrelated : Unexpected error: {:?}", err)
                }
            }
            sleep(Duration::from_millis(100));
        }
        assert!(!manager.is_running());
        //Verify exit code
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 0);
    }

    #[test]
    fn test_manager_pipeline_or_unsuccessful() {
        //Build pipeline
        let command: Vec<String> = vec![String::from("ping"), String::from("8.8.8.8.8.8.8.8.8.8")];
        let mut sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let command: Vec<String> = vec![String::from("echo"), String::from("bar")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Or,
        );
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Start
        assert!(manager.start().is_ok());
        //Wait for output messages
        let start_time: Instant = Instant::now(); //Timeout for 3 seconds
        //We expect 2 Output messages (One from stderr, one from stdout)
        let mut output_messages: u8 = 0;
        while output_messages < 2 {
            if start_time.elapsed().as_secs() >= 3 {
                panic!("test_manager_pipeline_or_unsuccessful : TaskManager timeout");
            }
            //Get message
            let inbox: Vec<TaskMessageRx> = manager.fetch_messages().unwrap();
            for message in inbox.iter() {
                match message {
                    TaskMessageRx::Output((stdout, stderr)) => {
                        output_messages += 1;
                        match output_messages {
                            1 => assert!(stderr.as_ref().is_some()), //The first is from stderr, the second from stdout
                            2 => assert_eq!(*stdout.as_ref().unwrap(), String::from("bar\n")),
                            _ => panic!("Received a 3rd message from task... That was unexpected...")
                        }
                        if output_messages == 1 {
                            println!("test_manager_pipeline_or_unsuccessful : Received message from task (stderr): '{}'", stderr.as_ref().unwrap());
                        } else {
                            println!("test_manager_pipeline_or_unsuccessful : Received message from task (stdout): '{}'", stdout.as_ref().unwrap());
                        }
                    },
                    TaskMessageRx::Error(err) => panic!("test_manager_pipeline_or_unsuccessful : Unexpected error: {:?}", err)
                }
            }
            sleep(Duration::from_millis(100));
        }
        //Verify exit code
        assert!(!manager.is_running());
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 0);
    }

    #[test]
    fn test_manager_pipeline_pipe_successful() {
        //Build pipeline
        let command: Vec<String> = vec![String::from("head"), String::from("-n"), String::from("1")];
        let mut sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let command: Vec<String> = vec![String::from("head"), String::from("-n"), String::from("1")];
        //Build pipeline
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Pipe,
        );
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Start
        assert!(manager.start().is_ok());
        //Wait for output messages
        let start_time: Instant = Instant::now(); //Timeout for 3 seconds
        //Wait 100ms
        sleep(Duration::from_millis(100));
        //We expect 1 Output messages
        let mut output_messages: u8 = 0;
        //Write to process
        assert!(manager.send_message(TaskMessageTx::Input(String::from("Hello world!\n"))).is_ok());
        //Wait for process; we'll receive the output from tail process
        while output_messages < 1 {
            if start_time.elapsed().as_secs() >= 3 {
                panic!("test_manager_pipeline_pipe_successful : TaskManager timeout");
            }
            //Get message
            let inbox: Vec<TaskMessageRx> = manager.fetch_messages().unwrap();
            for message in inbox.iter() {
                match message {
                    TaskMessageRx::Output((stdout, _stderr)) => {
                        output_messages += 1;
                        match output_messages {
                            1 => assert_eq!(*stdout.as_ref().unwrap(), String::from("Hello world!\n")),
                            _ => panic!("test_manager_pipeline_pipe_successful : Received a 2nd message from task... That was unexpected...")
                        }
                        println!("test_manager_pipeline_pipe_successful : Received message from task: '{}'", stdout.as_ref().unwrap());
                    },
                    TaskMessageRx::Error(err) => panic!("test_manager_pipeline_pipe_successful : Unexpected error: {:?}", err)
                }
            }
            sleep(Duration::from_millis(100));
        }
        //Verify exit code
        assert!(!manager.is_running());
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 0);
    }

    #[test]
    fn test_manager_pipeline_pipe_broken() {
        //Build pipeline
        let command: Vec<String> = vec![String::from("head"), String::from("-n"), String::from("1")];
        let mut sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let command: Vec<String> = vec![String::from("THISCOMMANDDOESNOTEXIST")];
        //Build pipeline
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Pipe,
        );
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Start
        assert!(manager.start().is_ok());
        //Wait for output messages
        let start_time: Instant = Instant::now(); //Timeout for 3 seconds
        //Wait 100ms
        sleep(Duration::from_millis(100));
        //We expect 1 Error message
        let mut output_messages: u8 = 0;
        //Wait for process; we'll receive the output from tail process
        while output_messages < 1 {
            if start_time.elapsed().as_secs() >= 3 {
                panic!("test_manager_pipeline_pipe_broken : TaskManager timeout");
            }
            //Get message
            let inbox: Vec<TaskMessageRx> = manager.fetch_messages().unwrap();
            for message in inbox.iter() {
                output_messages += 1;
                match message {
                    TaskMessageRx::Output((_stdout, _stderr)) => {
                        panic!("test_manager_pipeline_pipe_broken : Expected error, got Output")
                    },
                    TaskMessageRx::Error(err) => match output_messages {
                        1 => assert_eq!(err.code, TaskErrorCode::BrokenPipe),
                        _ => panic!("test_manager_pipeline_pipe_broken : Expected only 1 error, not more")
                    } 
                }
            }
            sleep(Duration::from_millis(100));
        }
        //Verify exit code
        assert!(!manager.is_running());
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 255);
    }

    #[test]
    fn test_manager_error() {
        //Build pipeline
        let command: Vec<String> = vec![String::from("THISCOMMANDWONTSTART")];
        let mut sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let command: Vec<String> = vec![String::from("echo"), String::from("bar")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Unrelated,
        );
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Start
        assert!(manager.start().is_ok());
        let start_time: Instant = Instant::now(); //Timeout for 3 seconds
        //Wait 100ms
        sleep(Duration::from_millis(100));
        let mut output_messages: u8 = 0;
        //Wait for 2 output messages, the first should be an error
        while output_messages < 2 {
            if start_time.elapsed().as_secs() >= 3 {
                panic!("test_manager_error : TaskManager timeout");
            }
            //Get message
            let inbox: Vec<TaskMessageRx> = manager.fetch_messages().unwrap();
            for message in inbox.iter() {
                output_messages += 1;
                match message {
                    TaskMessageRx::Output((stdout, _stderr)) => {
                        match output_messages {
                            2 => assert_eq!(*stdout.as_ref().unwrap(), String::from("bar\n")),
                            _ => panic!("test_manager_error : That was unexpected... only 2nd message should be output")
                        }
                        println!("test_manager_error : Received message from task (stdout): '{}'", stdout.as_ref().unwrap());
                    },
                    TaskMessageRx::Error(err) => match output_messages {
                        1 => assert_eq!(err.code, TaskErrorCode::CouldNotStartTask),
                        _ => panic!("That was unexpected... only 1st message should be error")
                    }
                }
            }
            sleep(Duration::from_millis(100));
        }
        //Verify exit code
        assert!(!manager.is_running());
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 0);
    }

    #[test]
    fn test_manager_write_stdin() {
        //Build pipeline
        let command: Vec<String> = vec![String::from("head"), String::from("-n"), String::from("1")];
        let sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Start
        assert!(manager.start().is_ok());
        //Wait for output messages
        let start_time: Instant = Instant::now(); //Timeout for 3 seconds
        //Wait 50ms
        sleep(Duration::from_millis(50));
        //We expect 1 Output messages
        let mut output_messages: u8 = 0;
        //Write to process
        assert!(manager.send_message(TaskMessageTx::Input(String::from("Hello world!\n"))).is_ok());
        //Wait for process
        while output_messages < 1 {
            if start_time.elapsed().as_secs() >= 3 {
                panic!("test_manager_write_stdin : TaskManager timeout");
            }
            //Get message
            let inbox: Vec<TaskMessageRx> = manager.fetch_messages().unwrap();
            for message in inbox.iter() {
                match message {
                    TaskMessageRx::Output((stdout, _stderr)) => {
                        output_messages += 1;
                        match output_messages {
                            1 => assert_eq!(*stdout.as_ref().unwrap(), String::from("Hello world!\n")),
                            _ => panic!("test_manager_write_stdin : Received a 2nd message from task... That was unexpected...")
                        }
                        println!("test_manager_write_stdin : Received message from task: '{}'", stdout.as_ref().unwrap());
                    },
                    TaskMessageRx::Error(err) => panic!("test_manager_write_stdin : Unexpected error: {:?}", err)
                }
            }
            sleep(Duration::from_millis(100));
        }
        //Verify exit code
        assert!(!manager.is_running());
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 0);
    }

    #[test]
    fn test_manager_kill() {
        //Build pipeline
        let command: Vec<String> = vec![String::from("cat")];
        let mut sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let command: Vec<String> = vec![String::from("echo"), String::from("bar")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Or,
        );
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Start
        assert!(manager.start().is_ok());
        //Wait for output messages
        let start_time: Instant = Instant::now(); //Timeout for 3 seconds
        //Wait 500ms
        sleep(Duration::from_millis(500));
        //Kill cat
        assert!(manager.send_message(TaskMessageTx::Kill).is_ok());
        //We expect 1 Output messages, since the first process was killed
        let mut output_messages: u8 = 0;
        while output_messages < 1 {
            if start_time.elapsed().as_secs() >= 3 {
                panic!("test_manager_kill : TaskManager timeout");
            }
            //Get message
            let inbox: Vec<TaskMessageRx> = manager.fetch_messages().unwrap();
            for message in inbox.iter() {
                match message {
                    TaskMessageRx::Output((stdout, _stderr)) => {
                        output_messages += 1;
                        match output_messages {
                            1 => assert_eq!(*stdout.as_ref().unwrap(), String::from("bar\n")),
                            _ => panic!("test_manager_kill : Received a 2nd message from task... That was unexpected...")
                        }
                        println!("test_manager_kill : Received message from task: '{}'", stdout.as_ref().unwrap());
                    },
                    TaskMessageRx::Error(err) => panic!("test_manager_kill : Unexpected error: {:?}", err)
                }
            }
            sleep(Duration::from_millis(100));
        }
        //Verify exit code
        assert!(!manager.is_running());
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 0);
    }

    #[test]
    fn test_manager_signal() {
        //Build pipeline
        let command: Vec<String> = vec![String::from("cat")];
        let mut sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let command: Vec<String> = vec![String::from("echo"), String::from("bar")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Or,
        );
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Start
        assert!(manager.start().is_ok());
        //Wait for output messages
        let start_time: Instant = Instant::now(); //Timeout for 3 seconds
        //Wait 500ms
        sleep(Duration::from_millis(500));
        //Send SIGABRT to cat process
        assert!(manager.send_message(TaskMessageTx::Signal(UnixSignal::Sigabrt)).is_ok());
        //We expect 1 Output messages, since the first process was killed
        let mut output_messages: u8 = 0;
        while output_messages < 1 {
            if start_time.elapsed().as_secs() >= 3 {
                panic!("test_manager_signal : TaskManager timeout");
            }
            //Get message
            let inbox: Vec<TaskMessageRx> = manager.fetch_messages().unwrap();
            for message in inbox.iter() {
                match message {
                    TaskMessageRx::Output((stdout, _stderr)) => {
                        output_messages += 1;
                        match output_messages {
                            1 => assert_eq!(*stdout.as_ref().unwrap(), String::from("bar\n")),
                            _ => panic!("test_manager_signal : Received a 2nd message from task... That was unexpected...")
                        }
                        println!("test_manager_signal : Received message from task: '{}'", stdout.as_ref().unwrap());
                    },
                    TaskMessageRx::Error(err) => panic!("test_manager_signal : Unexpected error: {:?}", err)
                }
            }
            sleep(Duration::from_millis(100));
        }
        //Verify exit code
        assert!(!manager.is_running());
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 0);
    }

    #[test]
    fn test_manager_terminate() {
        //Build pipeline
        let command: Vec<String> = vec![String::from("cat")];
        let mut sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let command: Vec<String> = vec![String::from("echo"), String::from("bar")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Or,
        );
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Start
        assert!(manager.start().is_ok());
        //Wait for output messages
        let start_time: Instant = Instant::now(); //Timeout for 3 seconds
        //Wait 500ms
        sleep(Duration::from_millis(200));
        //Terminate task manager
        assert!(manager.send_message(TaskMessageTx::Terminate).is_ok());
        sleep(Duration::from_millis(300));
        //Verify exit code
        assert!(!manager.is_running());
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 130);
    }

    #[test]
    fn test_manager_t1_and_t2_ur_t3() {
        //Build pipeline
        let command: Vec<String> = vec![String::from("echo"), String::from("foo")];
        let mut sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let command: Vec<String> = vec![String::from("echo"), String::from("bar")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::And,
        );
        let command: Vec<String> = vec![String::from("echo"), String::from("woff")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Unrelated,
        );
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Start
        assert!(manager.start().is_ok());
        //Wait for output messages
        let start_time: Instant = Instant::now(); //Timeout for 3 seconds
        //We expect 3 Output messages
        let mut output_messages: u8 = 0;
        while output_messages < 3 {
            if start_time.elapsed().as_secs() >= 3 {
                panic!("test_manager_t1_and_t2_ur_t3 : TaskManager timeout");
            }
            //Get message
            let inbox: Vec<TaskMessageRx> = manager.fetch_messages().unwrap();
            for message in inbox.iter() {
                match message {
                    TaskMessageRx::Output((stdout, _stderr)) => {
                        output_messages += 1;
                        match output_messages {
                            1 => assert_eq!(*stdout.as_ref().unwrap(), String::from("foo\n")),
                            2 => assert_eq!(*stdout.as_ref().unwrap(), String::from("bar\n")),
                            3 => assert_eq!(*stdout.as_ref().unwrap(), String::from("woff\n")),
                            _ => panic!("test_manager_t1_and_t2_ur_t3 : Received a 4th message from task... That was unexpected...")
                        }
                        println!("test_manager_t1_and_t2_ur_t3 : Received message from task: '{}'", stdout.as_ref().unwrap());
                    },
                    TaskMessageRx::Error(err) => panic!("test_manager_t1_and_t2_ur_t3 : Unexpected error: {:?}", err)
                }
            }
            sleep(Duration::from_millis(100));
        }
        //Verify exit code
        assert!(!manager.is_running());
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 0);
    }

    #[test]
    fn test_manager_not_t1_and_t2_ur_t3() {
        //Build pipeline
        let command: Vec<String> = vec![String::from("ping"), String::from("8.8.8.8.8.8.8.8.8")];
        let mut sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let command: Vec<String> = vec![String::from("echo"), String::from("bar")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::And,
        );
        let command: Vec<String> = vec![String::from("echo"), String::from("woff")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Unrelated,
        );
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Start
        assert!(manager.start().is_ok());
        //Wait for output messages
        let start_time: Instant = Instant::now(); //Timeout for 3 seconds
        //We expect 2 Output messages (1 stderr, 1 stdout)
        let mut output_messages: u8 = 0;
        while output_messages < 2 {
            if start_time.elapsed().as_secs() >= 3 {
                panic!("test_manager_not_t1_and_t2_ur_t3 : TaskManager timeout");
            }
            //Get message
            let inbox: Vec<TaskMessageRx> = manager.fetch_messages().unwrap();
            for message in inbox.iter() {
                match message {
                    TaskMessageRx::Output((stdout, stderr)) => {
                        output_messages += 1;
                        match output_messages {
                            1 => assert!(stderr.as_ref().is_some()),
                            2 => assert_eq!(*stdout.as_ref().unwrap(), String::from("woff\n")),
                            _ => panic!("test_manager_not_t1_and_t2_ur_t3 : Received a 3rd message from task... That was unexpected...")
                        }
                    },
                    TaskMessageRx::Error(err) => panic!("test_manager_not_t1_and_t2_ur_t3 : Unexpected error: {:?}", err)
                }
            }
            sleep(Duration::from_millis(100));
        }
        //Verify exit code
        assert!(!manager.is_running());
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 0);
    }

    #[test]
    fn test_manager_t1_or_t2_ur_t3() {
        //Build pipeline
        let command: Vec<String> = vec![String::from("echo"), String::from("foo")];
        let mut sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let command: Vec<String> = vec![String::from("echo"), String::from("bar")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Or,
        );
        let command: Vec<String> = vec![String::from("echo"), String::from("woff")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Unrelated,
        );
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Start
        assert!(manager.start().is_ok());
        //Wait for output messages
        let start_time: Instant = Instant::now(); //Timeout for 3 seconds
        //We expect 2 Output messages (from 1st and 3rd tasks)
        let mut output_messages: u8 = 0;
        while output_messages < 2 {
            if start_time.elapsed().as_secs() >= 3 {
                panic!("test_manager_t1_or_t2_ur_t3 : TaskManager timeout");
            }
            //Get message
            let inbox: Vec<TaskMessageRx> = manager.fetch_messages().unwrap();
            for message in inbox.iter() {
                match message {
                    TaskMessageRx::Output((stdout, _stderr)) => {
                        output_messages += 1;
                        match output_messages {
                            1 => assert_eq!(*stdout.as_ref().unwrap(), String::from("foo\n")),
                            2 => assert_eq!(*stdout.as_ref().unwrap(), String::from("woff\n")),
                            _ => panic!("test_manager_t1_or_t2_ur_t3 : Received a 3rd message from task... That was unexpected...")
                        }
                        println!("test_manager_t1_or_t2_ur_t3 : Received message from task: '{}'", stdout.as_ref().unwrap());
                    },
                    TaskMessageRx::Error(err) => panic!("test_manager_t1_or_t2_ur_t3 : Unexpected error: {:?}", err)
                }
            }
            sleep(Duration::from_millis(100));
        }
        //Verify exit code
        assert!(!manager.is_running());
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 0);
    }

    #[test]
    fn test_manager_not_t1_or_t2_ur_t3() {
        //Build pipeline
        let command: Vec<String> = vec![String::from("ping"), String::from("8.8.8.8.8.8.8.8.8")];
        let mut sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let command: Vec<String> = vec![String::from("echo"), String::from("bar")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Or,
        );
        let command: Vec<String> = vec![String::from("echo"), String::from("woff")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Unrelated,
        );
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Start
        assert!(manager.start().is_ok());
        //Wait for output messages
        let start_time: Instant = Instant::now(); //Timeout for 3 seconds
        //We expect 3 Output messages (1 stderr, 2 stdout)
        let mut output_messages: u8 = 0;
        while output_messages < 3 {
            if start_time.elapsed().as_secs() >= 3 {
                panic!("test_manager_not_t1_or_t2_ur_t3 : TaskManager timeout");
            }
            //Get message
            let inbox: Vec<TaskMessageRx> = manager.fetch_messages().unwrap();
            for message in inbox.iter() {
                match message {
                    TaskMessageRx::Output((stdout, stderr)) => {
                        output_messages += 1;
                        match output_messages {
                            1 => assert!(stderr.as_ref().is_some()),
                            2 => assert_eq!(*stdout.as_ref().unwrap(), String::from("bar\n")),
                            3 => assert_eq!(*stdout.as_ref().unwrap(), String::from("woff\n")),
                            _ => panic!("test_manager_not_t1_or_t2_ur_t3 : Received a 3rd message from task... That was unexpected...")
                        }
                    },
                    TaskMessageRx::Error(err) => panic!("test_manager_not_t1_or_t2_ur_t3 : Unexpected error: {:?}", err)
                }
            }
            sleep(Duration::from_millis(100));
        }
        //Verify exit code
        assert!(!manager.is_running());
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 0);
    }

    #[test]
    fn test_manager_t1_or_t2_and_t3() {
        //Build pipeline
        let command: Vec<String> = vec![String::from("echo"), String::from("foo")];
        let mut sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let command: Vec<String> = vec![String::from("echo"), String::from("bar")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Or,
        );
        let command: Vec<String> = vec![String::from("echo"), String::from("woff")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::And,
        );
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Start
        assert!(manager.start().is_ok());
        //Wait for output messages
        let start_time: Instant = Instant::now(); //Timeout for 3 seconds
        //We expect 2 Output messages (from 1st and 3rd tasks)
        let mut output_messages: u8 = 0;
        while output_messages < 2 {
            if start_time.elapsed().as_secs() >= 3 {
                panic!("test_manager_t1_or_t2_ur_t3 : TaskManager timeout");
            }
            //Get message
            let inbox: Vec<TaskMessageRx> = manager.fetch_messages().unwrap();
            for message in inbox.iter() {
                match message {
                    TaskMessageRx::Output((stdout, _stderr)) => {
                        output_messages += 1;
                        match output_messages {
                            1 => assert_eq!(*stdout.as_ref().unwrap(), String::from("foo\n")),
                            2 => assert_eq!(*stdout.as_ref().unwrap(), String::from("woff\n")),
                            _ => panic!("test_manager_t1_or_t2_ur_t3 : Received a 3rd message from task... That was unexpected...")
                        }
                        println!("test_manager_t1_or_t2_ur_t3 : Received message from task: '{}'", stdout.as_ref().unwrap());
                    },
                    TaskMessageRx::Error(err) => panic!("test_manager_t1_or_t2_ur_t3 : Unexpected error: {:?}", err)
                }
            }
            sleep(Duration::from_millis(100));
        }
        //Verify exit code
        assert!(!manager.is_running());
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 0);
    }

    #[test]
    fn test_manager_not_t1_or_t2_and_t3() {
        //Build pipeline
        let command: Vec<String> = vec![String::from("ping"), String::from("8.8.8.8.8.8.8.8.8")];
        let mut sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let command: Vec<String> = vec![String::from("echo"), String::from("bar")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Or,
        );
        let command: Vec<String> = vec![String::from("echo"), String::from("woff")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::And,
        );
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Start
        assert!(manager.start().is_ok());
        //Wait for output messages
        let start_time: Instant = Instant::now(); //Timeout for 3 seconds
        //We expect 3 Output messages (1 stderr, 2 stdout)
        let mut output_messages: u8 = 0;
        while output_messages < 3 {
            if start_time.elapsed().as_secs() >= 3 {
                panic!("test_manager_not_t1_or_t2_and_t3 : TaskManager timeout");
            }
            //Get message
            let inbox: Vec<TaskMessageRx> = manager.fetch_messages().unwrap();
            for message in inbox.iter() {
                match message {
                    TaskMessageRx::Output((stdout, stderr)) => {
                        output_messages += 1;
                        match output_messages {
                            1 => assert!(stderr.as_ref().is_some()),
                            2 => assert_eq!(*stdout.as_ref().unwrap(), String::from("bar\n")),
                            3 => assert_eq!(*stdout.as_ref().unwrap(), String::from("woff\n")),
                            _ => panic!("test_manager_not_t1_or_t2_and_t3 : Received a 3rd message from task... That was unexpected...")
                        }
                    },
                    TaskMessageRx::Error(err) => panic!("test_manager_not_t1_or_t2_and_t3 : Unexpected error: {:?}", err)
                }
            }
            sleep(Duration::from_millis(100));
        }
        //Verify exit code
        assert!(!manager.is_running());
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 0);
    }

    #[test]
    fn test_manager_thread_terminated_cause_of_dropped_receiver() {
        let command: Vec<String> = vec![String::from("cat")];
        let sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        //Instantiate task manager
        let mut manager: TaskManager = TaskManager::new(sample_task);
        //Start
        assert!(manager.start().is_ok());
        //Sleep for 100ms
        sleep(Duration::from_millis(100));
        //Set receiver to None, this will make the thread to fail
        manager.receiver = None;
        manager.sender = None;
        sleep(Duration::from_millis(200));
        //Verify exit code
        assert!(!manager.is_running());
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 255);
    }

}
