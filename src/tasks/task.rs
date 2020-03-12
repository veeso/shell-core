//! # Task
//!
//! `task` contains the implementation of Task

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

use super::process::{Process, ProcessError};
use super::{Redirection, Task, TaskError, TaskErrorCode, TaskRelation};
use crate::FileRedirectionType;

use std::fs::OpenOptions;
use std::io::Write;

impl Task {
    /// ## new
    ///
    /// Instantiate a new unrelated Task. To chain tasks, use new_pipeline()
    pub fn new(command: Vec<String>, stdout_redir: Redirection, stderr_redir: Redirection) -> Task {
        Task {
            command: command,
            stdout_redirection: stdout_redir,
            stderr_redirection: stderr_redir,
            process: None,
            relation: TaskRelation::Unrelated,
            next: None,
            exit_code: None,
        }
    }

    /// ## new_pipeline
    ///
    /// Add to Task a new task to create a pipeline
    pub fn new_pipeline(
        &mut self,
        next_command: Vec<String>,
        stdout_redir: Redirection,
        stderr_redir: Redirection,
        relation: TaskRelation,
    ) {
        //Set current relation to relation
        self.relation = relation;
        //Set new Task as Next Task
        self.next = Some(Box::new(Task::new(
            next_command,
            stdout_redir,
            stderr_redir,
        )));
    }

    /// ## start
    ///
    /// Start process
    /// NOTE: if the relation is Pipe, the next command is directly executed
    /// In pipes the processes are started in sequence from the last to the first one
    pub fn start(&mut self) -> Result<(), TaskError> {
        if self.relation == TaskRelation::Pipe {
            //Start next process
            if self.next.is_some() {
                if let Err(err) = self.next.as_mut().unwrap().start() {
                    return Err(err);
                }
            }
        }
        //After starting the pipe, execute this process
        self.process = match Process::exec(&self.command) {
            Ok(p) => Some(p),
            Err(_) => {
                return Err(TaskError::new(
                    TaskErrorCode::CouldNotStartTask,
                    format!("Could not start process {}", self.command[0].clone()),
                ))
            }
        };
        //Return OK
        Ok(())
    }

    /// read
    ///
    /// Read or redirect command output
    pub fn read(&mut self) -> Result<(String, String), TaskError> {
        match &mut self.process {
            None => Err(TaskError::new(
                TaskErrorCode::ProcessTerminated,
                String::from("Process is not running"),
            )),
            Some(p) => {
                match p.read() {
                    Ok((stdout, stderr)) => {
                        let mut res_stdout: String = String::new();
                        let mut res_stderr: String = String::new();
                        //Check redirections for stdout
                        if let Err(err) = self.redirect_output(
                            self.stdout_redirection.clone(),
                            stdout,
                            &mut res_stdout,
                            &mut res_stderr,
                        ) {
                            return Err(err);
                        }
                        //Check redirections fdr stderr
                        if let Err(err) = self.redirect_output(
                            self.stderr_redirection.clone(),
                            stderr,
                            &mut res_stdout,
                            &mut res_stderr,
                        ) {
                            return Err(err);
                        }
                        Ok((res_stdout, res_stderr))
                    }
                    Err(e) => Err(TaskError::new(
                        TaskErrorCode::IoError,
                        format!("Could not read from process: {}", e),
                    )),
                }
            }
        }
    }

    /// ### write
    /// 
    /// Write to process stdin
    pub fn write(&mut self, input: String) -> Result<(), TaskError> {
        match &mut self.process {
            None => Err(TaskError::new(
                TaskErrorCode::ProcessTerminated,
                String::from("Process is not running"),
            )),
            Some(p) => {
                match p.write(input) {
                    Ok(()) => Ok(()),
                    Err(err) => Err(TaskError::new(TaskErrorCode::IoError, format!("Could not write to process stdin {}", err)))
                }
            }
        }
    }

    //TODO: kill
    //TODO: signal

    /// ### is_running
    /// 
    /// Returns whether the process is running. If the process has terminated, the exitcode will be set
    pub fn is_running(&mut self) -> bool {
        match &mut self.process {
            Some(p) => {
                match p.is_running() {
                    true => true,
                    false => {
                        self.exit_code = p.exit_status.clone();
                        false
                    }
                }
            },
            None => false
        }
    }

    /// ### get_exitcode
    /// 
    /// Return task's exitcode
    pub fn get_exitcode(&self) -> Option<u8> {
        self.exit_code.clone()
    }

    /// ### redirect_output
    ///
    /// Handle output redirections in a single method
    fn redirect_output(
        &self,
        redirection: Redirection,
        output: Option<String>,
        stdout: &mut String,
        stderr: &mut String,
    ) -> Result<(), TaskError> {
        match redirection {
            Redirection::Stdout => {
                if output.is_some() {
                    stdout.push_str(&output.unwrap());
                }
            }
            Redirection::Stderr => {
                if output.is_some() {
                    stderr.push_str(&output.unwrap());
                }
            }
            Redirection::File(file, file_mode) => {
                if output.is_some() {
                    return self.redirect_to_file(file, file_mode, output.unwrap());
                }
            }
        }
        Ok(())
    }

    /// ### redirect_to_file
    ///
    /// Redirect a certain output to a certain file
    fn redirect_to_file(
        &self,
        file: String,
        file_mode: FileRedirectionType,
        out: String,
    ) -> Result<(), TaskError> {
        match OpenOptions::new()
            .create(true)
            .append(file_mode == FileRedirectionType::Append)
            .truncate(file_mode == FileRedirectionType::Truncate)
            .open(file.as_str())
        {
            Ok(mut f) => {
                if let Err(e) = writeln!(f, "{}", out) {
                    Err(TaskError::new(
                        TaskErrorCode::IoError,
                        format!("Could not write to file {}: {}", file, e),
                    ))
                } else {
                    Ok(())
                }
            }
            Err(e) => Err(TaskError::new(
                TaskErrorCode::IoError,
                format!("Could not open file {}: {}", file, e),
            )),
        }
    }
}
