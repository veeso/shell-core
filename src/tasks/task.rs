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

use super::process::Process;
use super::{Redirection, Task, TaskError, TaskErrorCode, TaskRelation};
use crate::{FileRedirectionType, UnixSignal};

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
    /// Add to Task a new task
    pub fn new_pipeline(
        &mut self,
        next_command: Vec<String>,
        stdout_redir: Redirection,
        stderr_redir: Redirection,
        relation: TaskRelation,
    ) {
        //If next is None, set Next as new Task, otherwise pass new task to the next of the next etc...
        match &mut self.next {
            None => self.next = {
                //Set current relation to relation
                self.relation = relation;
                Some(Box::new(Task::new(next_command, stdout_redir, stderr_redir)))
            },
            Some(task) => task.new_pipeline(next_command, stdout_redir, stderr_redir, relation)
        }
    }

    /// ## start
    ///
    /// Start process
    /// NOTE: if the relation is Pipe, the next command is directly executed
    /// In pipes the processes are started in sequence from the last to the first one
    pub fn start(&mut self) -> Result<(), TaskError> {
        if self.process.is_some() {
            return Err(TaskError::new(TaskErrorCode::AlreadyRunning, String::from("Could not start process since it is already running")))
        }
        if self.relation == TaskRelation::Pipe {
            //Start next process
            if self.next.is_some() {
                if let Err(_) = self.next.as_mut().unwrap().start() {
                    return Err(TaskError::new(TaskErrorCode::BrokenPipe, String::from("Failed to start next process in the pipeline")));
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
    pub fn read(&mut self) -> Result<(Option<String>, Option<String>), TaskError> {
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
                        if let Err(err) = self.redirect_output(self.stdout_redirection.clone(), stdout, &mut res_stdout, &mut res_stderr) {
                            return Err(err);
                        }
                        //Check redirections fdr stderr
                        if let Err(err) = self.redirect_output(self.stderr_redirection.clone(), stderr, &mut res_stdout, &mut res_stderr) {
                            return Err(err);
                        }
                        let res_stdout: Option<String> = match res_stdout.len() {
                            0 => None,
                            _ => Some(res_stdout),
                        };
                        let res_stderr: Option<String> = match res_stderr.len() {
                            0 => None,
                            _ => Some(res_stderr),
                        };
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
            Some(p) => match p.write(input) {
                Ok(()) => Ok(()),
                Err(err) => Err(TaskError::new(
                    TaskErrorCode::IoError,
                    format!("Could not write to process stdin {}", err),
                )),
            },
        }
    }

    /// ### kill
    ///
    /// Kill running task
    pub fn kill(&mut self) -> Result<(), TaskError> {
        match &mut self.process {
            None => Err(TaskError::new(
                TaskErrorCode::ProcessTerminated,
                String::from("Process is not running"),
            )),
            Some(p) => match p.kill() {
                Ok(()) => Ok(()),
                Err(()) => Err(TaskError::new(
                    TaskErrorCode::KillError,
                    String::from("It was not possible to kill process"),
                )),
            },
        }
    }

    /// ### raise
    ///
    /// Raise a signal on the process
    pub fn raise(&mut self, signal: UnixSignal) -> Result<(), TaskError> {
        match &mut self.process {
            None => Err(TaskError::new(
                TaskErrorCode::ProcessTerminated,
                String::from("Process is not running"),
            )),
            Some(p) => match p.raise(signal) {
                Ok(()) => Ok(()),
                Err(()) => Err(TaskError::new(
                    TaskErrorCode::KillError,
                    String::from("It was not possible to send signal to process"),
                )),
            },
        }
    }

    /// ### is_running
    ///
    /// Returns whether the process is running. If the process has terminated, the exitcode will be set
    pub fn is_running(&mut self) -> bool {
        match &mut self.process {
            Some(p) => match p.is_running() {
                true => true,
                false => {
                    self.exit_code = p.exit_status.clone();
                    false
                }
            },
            None => false,
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
    /// NOTE: This method redirects output to pipes too
    fn redirect_output(&mut self, redirection: Redirection, output: Option<String>, stdout: &mut String, stderr: &mut String) -> Result<(), TaskError> {
        match redirection {
            Redirection::Stdout => {
                if output.is_some() {
                    //If relation is Pipe, write output to NEXT TASK, otherwise push to stdout string
                    if self.relation == TaskRelation::Pipe {
                        match self.next.as_mut().unwrap().write(output.unwrap()) {
                            Ok(()) => return Ok(()),
                            Err(err) => return Err(TaskError::new(TaskErrorCode::BrokenPipe, err.message))
                        }
                    } else {
                        stdout.push_str(&output.unwrap());
                    }
                }
            },
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
    fn redirect_to_file(&self, file: String, file_mode: FileRedirectionType, out: String) -> Result<(), TaskError> {
        match OpenOptions::new().create(true).append(file_mode == FileRedirectionType::Append).truncate(file_mode == FileRedirectionType::Truncate).open(file.as_str()) {
            Ok(mut f) => {
                if let Err(e) = write!(f, "{}", out) {
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

//@! Module Test

#[cfg(test)]
mod tests {

    use super::*;

    use std::fs::File;
    use std::io::Read;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn test_task_new() {
        let command: Vec<String> = vec![String::from("echo"), String::from("foobar")];
        let task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        //Verify constructor
        assert!(task.exit_code.is_none());
        assert!(task.process.is_none());
        assert!(task.next.is_none());
        assert_eq!(task.relation, TaskRelation::Unrelated);
        assert_eq!(task.stderr_redirection, Redirection::Stderr);
        assert_eq!(task.stdout_redirection, Redirection::Stdout);
        assert_eq!(task.command[0], String::from("echo"));
        assert_eq!(task.command[1], String::from("foobar"));
    }

    #[test]
    fn test_task_start_run() {
        let command: Vec<String> = vec![String::from("echo"), String::from("foobar")];
        let mut task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        //Start process
        assert!(task.start().is_ok());
        //Wait 100ms
        sleep(Duration::from_millis(100));
        //Read stdout/stderr
        let (stdout, stderr) = task.read().unwrap();
        //Verify stdout
        assert_eq!(stdout.unwrap(), String::from("foobar\n"));
        //Verify stderr
        assert!(stderr.is_none());
        //Process should not be running anymore
        assert!(!task.is_running());
        //Get exitcode
        assert_eq!(task.get_exitcode().unwrap(), 0);
    }

    #[test]
    fn test_task_start_failed() {
        let command: Vec<String> = vec![String::from("thiscommandfails")];
        let mut task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        //Start process
        assert_eq!(
            task.start().err().unwrap().code,
            TaskErrorCode::CouldNotStartTask
        );
    }

    #[test]
    fn test_task_run_twice() {
        let command: Vec<String> = vec![String::from("cat")];
        let mut task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        //Start process
        assert!(task.start().is_ok());
        //Wait 100ms
        sleep(Duration::from_millis(100));
        //Try to Start process another time
        assert_eq!(task.start().err().unwrap().code, TaskErrorCode::AlreadyRunning);
        //Process should be still running
        assert!(task.is_running());
        //Kill process
        assert!(task.kill().is_ok());
        //Verify process terminated
        assert!(!task.is_running());
        //Exit code should be 9
        assert_eq!(task.get_exitcode().unwrap(), 9);
    }

    #[test]
    fn test_task_write() {
        let command: Vec<String> = vec![String::from("cat")];
        let mut task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        //Start process
        assert!(task.start().is_ok());
        //Write to process
        assert!(task.write(String::from("hi there!\n")).is_ok());
        //Wait 100ms
        sleep(Duration::from_millis(100));
        //Read process output
        let (stdout, stderr) = task.read().unwrap();
        //Verify stdout
        assert_eq!(stdout.unwrap(), String::from("hi there!\n"));
        //Verify stderr
        assert!(stderr.is_none());
        //Process should be still running
        assert!(task.is_running());
        //Kill process
        assert!(task.kill().is_ok());
        //Verify process terminated
        assert!(!task.is_running());
        //Exit code should be 9
        assert_eq!(task.get_exitcode().unwrap(), 9);
    }

    #[test]
    fn test_task_kill() {
        let command: Vec<String> = vec![String::from("yes")];
        let mut task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        //Start process
        assert!(task.start().is_ok());
        //Wait 100ms
        sleep(Duration::from_millis(100));
        //Process should be still running
        assert!(task.is_running());
        //Kill process
        assert!(task.raise(UnixSignal::Sigint).is_ok());
        //Verify process terminated
        assert!(!task.is_running());
        //Exit code should be 9
        assert_eq!(task.get_exitcode().unwrap(), 2);
    }

    #[test]
    fn test_task_pipeline_simple() {
        let command: Vec<String> = vec![String::from("echo"), String::from("foo")];
        let mut task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        //Add pipe
        let command: Vec<String> = vec![String::from("echo"), String::from("bar")];
        task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::And,
        );
        assert_eq!(task.relation, TaskRelation::And);
        //Verify next is something
        assert!(task.next.is_some());
        //Start process
        assert!(task.start().is_ok());
        //Wait 100ms
        sleep(Duration::from_millis(100));
        //Read stdout/stderr
        let (stdout, stderr) = task.read().unwrap();
        //Verify stdout
        assert_eq!(stdout.unwrap(), String::from("foo\n"));
        //Verify stderr
        assert!(stderr.is_none());
        //Process should not be running anymore
        assert!(!task.is_running());
        //Get exitcode
        assert_eq!(task.get_exitcode().unwrap(), 0);
        //Start next process
        let mut task: Task = *task.next.unwrap();
        //Verify next of second process is None
        assert_eq!(task.relation, TaskRelation::Unrelated);
        //Start second process
        assert!(task.start().is_ok());
        //Wait 100ms
        sleep(Duration::from_millis(100));
        //Read stdout/stderr
        let (stdout, stderr) = task.read().unwrap();
        //Verify stdout
        assert_eq!(stdout.unwrap(), String::from("bar\n"));
        //Verify stderr
        assert!(stderr.is_none());
        //Process should not be running anymore
        assert!(!task.is_running());
        //Get exitcode
        assert_eq!(task.get_exitcode().unwrap(), 0);
    }

    #[test]
    fn test_task_pipeline_with_3_tasks() {
        let command: Vec<String> = vec![String::from("echo"), String::from("foo")];
        let mut task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        //Add pipe
        let command: Vec<String> = vec![String::from("echo"), String::from("bar")];
        task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::And,
        );
        let command: Vec<String> = vec![String::from("echo"), String::from("pippo")];
        task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::And,
        );
        assert_eq!(task.relation, TaskRelation::And);
        //Verify next is something
        assert!(task.next.is_some());
        //Start process
        assert!(task.start().is_ok());
        //Wait 100ms
        sleep(Duration::from_millis(100));
        //Read stdout/stderr
        let (stdout, stderr) = task.read().unwrap();
        //Verify stdout
        assert_eq!(stdout.unwrap(), String::from("foo\n"));
        //Verify stderr
        assert!(stderr.is_none());
        //Process should not be running anymore
        assert!(!task.is_running());
        //Get exitcode
        assert_eq!(task.get_exitcode().unwrap(), 0);
        //@! Start SECOND process
        let mut task: Task = *task.next.unwrap();
        //Verify next of second process is None
        assert_eq!(task.relation, TaskRelation::And);
        //Start second process
        assert!(task.start().is_ok());
        //Wait 100ms
        sleep(Duration::from_millis(100));
        //Read stdout/stderr
        let (stdout, stderr) = task.read().unwrap();
        //Verify stdout
        assert_eq!(stdout.unwrap(), String::from("bar\n"));
        //Verify stderr
        assert!(stderr.is_none());
        //Process should not be running anymore
        assert!(!task.is_running());
        //Get exitcode
        assert_eq!(task.get_exitcode().unwrap(), 0);
        //@! Start THIRD process
        let mut task: Task = *task.next.unwrap();
        //Verify next of second process is None
        assert_eq!(task.relation, TaskRelation::Unrelated);
        //Start second process
        assert!(task.start().is_ok());
        //Wait 100ms
        sleep(Duration::from_millis(100));
        //Read stdout/stderr
        let (stdout, stderr) = task.read().unwrap();
        //Verify stdout
        assert_eq!(stdout.unwrap(), String::from("pippo\n"));
        //Verify stderr
        assert!(stderr.is_none());
        //Process should not be running anymore
        assert!(!task.is_running());
        //Get exitcode
        assert_eq!(task.get_exitcode().unwrap(), 0);
    }

    #[test]
    fn test_task_pipeline_with_pipe_mode() {
        let command: Vec<String> = vec![String::from("echo"), String::from("foobar")];
        let mut task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        //Add pipe
        let command: Vec<String> = vec![String::from("head"), String::from("-n"), String::from("1")];
        task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Pipe,
        );
        assert_eq!(task.relation, TaskRelation::Pipe);
        //Verify next is something
        assert!(task.next.is_some());
        //Start process
        assert!(task.start().is_ok());
        //Wait 100ms
        sleep(Duration::from_millis(100));
        //Read stdout/stderr; output should be redirected to cat process, so should be None
        let (stdout, stderr) = task.read().unwrap();
        assert!(stdout.is_none());
        assert!(stderr.is_none());
        //Process should not be running anymore
        assert!(!task.is_running());
        //Get exitcode
        assert_eq!(task.get_exitcode().unwrap(), 0);
        let mut task: Task = *task.next.unwrap();
        //Verify next task output is foobar
        let (stdout, _stderr) = task.read().unwrap();
        assert_eq!(stdout.unwrap(), String::from("foobar\n"));
        //Wait 500ms
        sleep(Duration::from_millis(500));
        //2nd Process should not be running anymore
        assert!(!task.is_running());
        //Get exitcode
        assert_eq!(task.get_exitcode().unwrap(), 0);
    }

    #[test]
    fn test_task_pipeline_with_pipe_mode_brokenpipe() {
        let command: Vec<String> = vec![String::from("echo"), String::from("foo")];
        let mut task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        //Add pipe
        let command: Vec<String> = vec![String::from("echo"), String::from("bar")];
        task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Pipe,
        );
        assert_eq!(task.relation, TaskRelation::Pipe);
        //Verify next is something
        assert!(task.next.is_some());
        //Start process
        assert!(task.start().is_ok());
        //Wait 500ms
        sleep(Duration::from_millis(500));
        //@! Since the second process has terminated before the first, it'll return broken pipe
        assert_eq!(task.read().err().unwrap().code, TaskErrorCode::BrokenPipe);
        //Process should not be running anymore
        assert!(!task.is_running());
        //Get exitcode
        assert_eq!(task.get_exitcode().unwrap(), 0);
    }

    #[test]
    fn test_task_pipeline_with_pipe_mode_failed() {
        let command: Vec<String> = vec![String::from("echo"), String::from("foo")];
        let mut task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        //Add pipe
        let command: Vec<String> = vec![String::from("THISCOMMANDDOESNOTEXIST")];
        task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Pipe,
        );
        assert_eq!(task.relation, TaskRelation::Pipe);
        //Verify next is something
        assert!(task.next.is_some());
        //Start process
        assert_eq!(task.start().err().unwrap().code, TaskErrorCode::BrokenPipe);
        //Wait 100ms
        sleep(Duration::from_millis(100));
        //Process should not be running
        assert!(!task.is_running());
    }

    #[test]
    fn test_task_redirect_stdout_to_file() {
        let command: Vec<String> = vec![String::from("echo"), String::from("foobar")];
        let mut tmpfile = create_tmpfile();
        let tmpfile_path: String = String::from(tmpfile.path().to_str().unwrap());
        let mut task: Task = Task::new(
            command,
            Redirection::File(tmpfile_path, FileRedirectionType::Append),
            Redirection::Stderr,
        );
        //Start second process
        assert!(task.start().is_ok());
        //Wait 100ms
        sleep(Duration::from_millis(100));
        //Read stdout/stderr
        let (stdout, stderr) = task.read().unwrap();
        //Stdout and stderr should be both none (Since output has been redirected to file)
        assert!(stdout.is_none());
        assert!(stderr.is_none());
        //Process should not be running anymore
        assert!(!task.is_running());
        //Get exitcode
        assert_eq!(task.get_exitcode().unwrap(), 0);
        //Read file
        let output: String = read_file(&mut tmpfile);
        assert_eq!(output, String::from("foobar\n"));
    }

    #[test]
    fn test_task_stderr() {
        let command: Vec<String> = vec![
            String::from("ping"),
            String::from("8.8.8.8.8.8.8.8.8.8.8.8.8.8.8"),
        ];
        let mut task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        //Start second process
        assert!(task.start().is_ok());
        //Wait 100ms
        sleep(Duration::from_millis(100));
        //Read stdout/stderr
        let (stdout, stderr) = task.read().unwrap();
        //Stdout and stderr should be both none (Since output has been redirected to file)
        assert!(stdout.is_none());
        assert!(stderr.is_some());
        assert!(stderr.unwrap().len() > 4); //Should at least be ping:
        sleep(Duration::from_millis(100));
        //Process should not be running anymore
        assert!(!task.is_running());
        //Get exitcode
        assert!(task.get_exitcode().unwrap() != 0); //Exitcode won't be 0
    }

    #[test]
    fn test_task_redirect_stderr_to_file() {
        let command: Vec<String> = vec![
            String::from("ping"),
            String::from("8.8.8.8.8.8.8.8.8.8.8.8.8.8.8"),
        ];
        let mut tmpfile = create_tmpfile();
        let tmpfile_path: String = String::from(tmpfile.path().to_str().unwrap());
        let mut task: Task = Task::new(
            command,
            Redirection::Stdout,
            Redirection::File(tmpfile_path, FileRedirectionType::Append),
        );
        //Start second process
        assert!(task.start().is_ok());
        //Wait 100ms
        sleep(Duration::from_millis(100));
        //Read stdout/stderr
        let (stdout, stderr) = task.read().unwrap();
        //Stdout and stderr should be both none (Since output has been redirected to file)
        assert!(stdout.is_none());
        assert!(stderr.is_none());
        //Process should not be running anymore
        sleep(Duration::from_millis(100));
        assert!(!task.is_running());
        //Get exitcode
        assert!(task.get_exitcode().unwrap() != 0); //Exitcode won't be 0
                                                    //Read file
        let output: String = read_file(&mut tmpfile);
        println!("Stderr output: {}", output);
        assert!(output.len() > 4); //Should at least be ping:
    }

    #[test]
    fn test_task_kill_not_running() {
        let command: Vec<String> = vec![String::from("yes")];
        let mut task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        //OOps, I forgot to start process
        //Process should be still running
        assert!(!task.is_running());
        //Kill process
        assert_eq!(
            task.raise(UnixSignal::Sigint).err().unwrap().code,
            TaskErrorCode::ProcessTerminated
        );
        assert_eq!(
            task.kill().err().unwrap().code,
            TaskErrorCode::ProcessTerminated
        );
        //Exit code should be 9
        assert!(task.get_exitcode().is_none());
    }

    fn create_tmpfile() -> tempfile::NamedTempFile {
        tempfile::NamedTempFile::new().unwrap()
    }

    fn read_file(tmpfile: &mut tempfile::NamedTempFile) -> String {
        let file: &mut File = tmpfile.as_file_mut();
        let mut out: String = String::new();
        assert!(file.read_to_string(&mut out).is_ok());
        out
    }
}
