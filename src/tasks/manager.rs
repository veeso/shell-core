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

use super::{Task, TaskError, TaskErrorCode, TaskManager, TaskRelation};

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
            //Start process
            if let Err(err) = task.start() {
                return Err(err)
            }
            loop {
                //Ir process is running handle I/O
                if task.is_running() {
                    //TODO: Read from process
                    //TODO: Handle TX receiver
                    //If process is running, sleep for 100ms
                    thread::sleep(Duration::from_millis(100));
                } else { //@! Otherwise handle next process in pipeline
                    //@! Start next process or break from loop if pipeline has terminated
                    match task.next {
                        Some(t) => {
                            //Start next task based on relation
                            match task.relation {
                                TaskRelation::And => { //Start new process ONLY if this process' exitcode was SUCCESSFUL
                                    if task.exit_code.unwrap_or(255) == 0 {
                                        task = *t;
                                        //Start next process
                                        if let Err(err) = task.start() {
                                            return Err(err)
                                        }
                                    } else {
                                        //Else break from loop
                                        break;
                                    }
                                },
                                TaskRelation::Or => { //Start new process ONLY if this process' exitcode was UNSUCCESSFUL
                                    task = *t;
                                    if task.exit_code.unwrap_or(255) != 0 { //@! OR relation was UNSUCCESSFUL
                                        //Start next process
                                        if let Err(err) = task.start() {
                                            return Err(err)
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
                                        return Err(err)
                                    }
                                }
                            }
                        },
                        None => break //No remaining process in pipeline, terminate
                    }
                }
            }
            //Return exitcode
            Ok(task.exit_code.unwrap_or(255))
        }));
        Ok(())
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
            self.m_loop.take().map(thread::JoinHandle::join).unwrap().unwrap()
        } else {
            Err(TaskError::new(TaskErrorCode::ProcessTerminated, String::from("Could not join task manager, since process has never been started")))
        }
    }

//sender: Option<mpsc::Sender<TaskMessageRx>>, //Sends Task messages

}
