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
        //Get process out from TaskManager
        let mut task = self.next.take().unwrap();
        //Start thread
        self.m_loop = Some(thread::spawn(move || {
            TaskManager::run(task, tx_receiver, rx_sender)
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
    fn run(mut task: Task, tx_receiver: mpsc::Receiver<TaskMessageTx>, rx_sender: mpsc::Sender<TaskMessageRx>) -> u8 {
        //Start process
        if let Err(err) = task.start() {
            //Report error
            if rx_sender.send(TaskMessageRx::Error(err)).is_err() {
                return 255 //The other end hung up, so  terminate the thread
            }
        }
        let mut last_exit_code: u8 = 255;
        loop {
            //If process is running handle I/O
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
                                            return 255 //The other end hung up, so  terminate the thread
                                        }
                                    }
                                },
                                TaskMessageTx::Kill => {
                                    //Try Kill process
                                    if let Err(err) = task.kill() {
                                        //Report error in killing process
                                        if rx_sender.send(TaskMessageRx::Error(err)).is_err() {
                                            return 255 //The other end hung up, so  terminate the thread
                                        }
                                    }
                                },
                                TaskMessageTx::Signal(signal) => {
                                    //Try Send signal to process
                                    if let Err(err) = task.raise(signal) {
                                        //Report error in signaling process
                                        if rx_sender.send(TaskMessageRx::Error(err)).is_err() {
                                            return 255 //The other end hung up, so  terminate the thread
                                        }
                                    }
                                }
                            }
                        },
                        Err(recv_error) => {
                            match recv_error {
                                mpsc::TryRecvError::Empty => break, //No more messages, break from loop
                                _ => {
                                    //Kill process and return, the endpoint hung up
                                    let _ = task.kill();
                                    return 255
                                }
                            }
                        }
                    }
                }
                //Read from process
                match task.read() {
                    Ok((stdout, stderr)) => {
                        //Send stdout and stderr
                        if rx_sender.send(TaskMessageRx::Output((stdout, stderr))).is_err() {
                            return 255 //The other end hung up, so  terminate the thread
                        }
                    },
                    Err(err) => { //Report error
                        //Report error
                        if rx_sender.send(TaskMessageRx::Error(err)).is_err() {
                            return 255 //The other end hung up, so  terminate the thread
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
                last_exit_code = task.get_exitcode().unwrap_or(last_exit_code);
                match task.next {
                    Some(t) => {
                        //Start next task based on relation
                        match task.relation {
                            TaskRelation::And => { //Start new process ONLY if this process' exitcode was SUCCESSFUL
                                task = *t;
                                if task.exit_code.unwrap_or(255) == 0 {
                                    //Start next process
                                    if let Err(err) = task.start() {
                                        //Report error in starting process
                                        if rx_sender.send(TaskMessageRx::Error(err)).is_err() {
                                            return 255 //The other end hung up, so  terminate the thread
                                        }
                                    }
                                }
                            },
                            TaskRelation::Or => { //Start new process ONLY if this process' exitcode was UNSUCCESSFUL
                                task = *t;
                                if task.exit_code.unwrap_or(255) != 0 { //@! OR relation was UNSUCCESSFUL
                                    //Start next process
                                    if let Err(err) = task.start() {
                                        //Report error in starting process
                                        if rx_sender.send(TaskMessageRx::Error(err)).is_err() {
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
                                //FIXME: maybe the output should be re-read here
                                task = *t;
                            },
                            TaskRelation::Unrelated => {
                                //Just start next process
                                task = *t;
                                if let Err(err) = task.start() {
                                    //Report error in starting process
                                    if rx_sender.send(TaskMessageRx::Error(err)).is_err() {
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
        //Return exitcode
        task.exit_code.unwrap_or(last_exit_code)
    }

}

//@! Module Test

#[cfg(test)]
mod tests {

    use super::*;
    use crate::Redirection;

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
                    TaskMessageRx::Output((stdout, stderr)) => {
                        assert_eq!(*stdout.as_ref().unwrap(), String::from("foobar\n"));
                        println!("test_manager_one_task : Received message from task: '{}'", stdout.as_ref().unwrap());
                        message_recv = true;
                        break;
                    },
                    TaskMessageRx::Error(err) => panic!("test_manager_one_task : Unexpected error: {:?}", err)
                }
            }
            sleep(Duration::from_millis(100));
        }
        //Verify exit code
        let rc: u8 = manager.join().unwrap();
        assert_eq!(rc, 0);
    }

}
