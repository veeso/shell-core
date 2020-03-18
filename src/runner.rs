//! # Runner
//!
//! `runner` provides the implementations for ShellRunner.
//! This module takes care of executing the ShellExpressions

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

extern crate glob;

use crate::{FileRedirectionType, Redirection, ShellCore, ShellError, ShellExpression, ShellRunner, ShellStream, ShellStreamMessage, TaskManager, Task, UserStreamMessage};
use crate::tasks::{TaskError, TaskErrorCode, TaskMessageRx, TaskMessageTx, TaskRelation};

use glob::glob;
use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::thread::sleep;

/// ## TaskChain
/// 
/// A TaskChain is a wrapper for tasks which is used by the Exec statement processor, since Tasks could be function which cannot be handled by
/// the TaskManager
#[derive(std::fmt::Debug)]
struct TaskChain {
    pub task: Option<Task>,
    pub function: Option<Function>,
    pub prev_relation: TaskRelation,
    pub next_relation: TaskRelation,
    pub next: Option<Box<TaskChain>>
}

/// ## Function
/// 
/// A Function is the wrapper for a function inside a TaskChain
#[derive(std::fmt::Debug)]
struct Function {
    pub expression: ShellExpression,
    pub redirection: Redirection,
}

impl ShellRunner {

    /// ### new
    /// 
    /// Instantiate a new ShellRunner
    pub(crate) fn new() -> ShellRunner {
        ShellRunner {
            buffer: None,
            exit_flag: None
        }
    }

    /// ### run
    /// 
    /// Run a Shell Expression. This function basically iterates over the Shell Expression's statements. 
    /// Most of the statements have a function where they're executed, but some of them doesn't require one (e.g. break, continue, return, exit) which
    /// may be executed and treated inside another function or inside run function.
    /// The exec statements must be resolved in their executable (e.g. functions/alias inside resolve_exec function)
    /// NOTE: this function may become recursive, in case of execution of a function
    pub(crate) fn run(&mut self, core: &mut ShellCore, expression: ShellExpression) -> u8 {
        let (rc, _): (u8, String) = self.run_expression(core, expression);
        rc
    }

    //@! Statements

    /// ### alias
    /// 
    /// Execute alias statement
    fn alias(&self, core: &mut ShellCore, name: String, command: String) {
        core.alias_set(name, command);
    }

    /// ### change_directory
    /// 
    /// Execute cd statement
    fn change_directory(&self, core: &mut ShellCore, path: PathBuf) -> Result<(), ShellError> {
        core.change_directory(path)
    }

    /// ### dirs
    /// 
    /// Returns the directories in the core stack
    fn dirs(&self, core: &mut ShellCore) -> VecDeque<PathBuf> {
        core.dirs()
    }

    /// ### exec
    /// 
    /// Executes through the task manager a Task
    fn exec(&mut self, core: &mut ShellCore, task: Task) -> Result<u8, ShellError> {
        //Execution flags
        let mut brutally_terminated: bool = false;
        let mut relation_satisfied: bool = true;
        //Create command chain from Task
        let mut chain: TaskChain = self.chain_task(core, task);
        let mut rc: u8 = 0;
        //Iterate over task chain
        loop {
            if relation_satisfied { //Only if relation is satisfied
                //Match chain block
                if let Some(task) = chain.task { //@! TaskManager
                    //Instantiate a new task manager
                    let mut task_manager: TaskManager = TaskManager::new(task);
                    //Execute task
                    if let Err(err) = task_manager.start() {
                        if !core.sstream.send(ShellStreamMessage::Error(ShellError::TaskError(err))) {
                            break; //Endpoint hung up
                        }
                    }
                    //write buffer to task
                    if let Some(input) = &self.buffer {
                        let _ = task_manager.send_message(TaskMessageTx::Input(input.to_string()));
                    }
                    self.buffer = None;
                    //Iterate until task manager is running
                    while task_manager.is_running() {
                        //Fetch messages
                        match task_manager.fetch_messages() {
                            Ok(inbox) => {
                                //Iterate over inbox
                                for message in inbox.iter() {
                                    //Match message type and report to shell stream
                                    match message {
                                        TaskMessageRx::Error(err) => {
                                            let _ = core.sstream.send(ShellStreamMessage::Error(ShellError::TaskError(err.clone())));
                                        },
                                        TaskMessageRx::Output((stdout, stderr)) => {
                                            if stdout.is_some() || stderr.is_some() {
                                                let _ = core.sstream.send(ShellStreamMessage::Output((stdout.clone(), stderr.clone())));
                                            }
                                        }
                                    }
                                }
                            },
                            Err(err) => {
                                //Report error and break
                                let _ = core.sstream.send(ShellStreamMessage::Error(ShellError::TaskError(err)));
                                //Terminate task manager
                                let _ = task_manager.send_message(TaskMessageTx::Terminate);
                                break;
                            }
                        }
                        //@! fetch user messages
                        match core.sstream.receive() {
                            Ok(inbox) => {
                                //Iterate over inbox
                                for message in inbox.iter() {
                                    match message {
                                        UserStreamMessage::Input(stdin) => {
                                            //Write stdin
                                            if let Err(err) = task_manager.send_message(TaskMessageTx::Input(stdin.clone())) {
                                                core.sstream.send(ShellStreamMessage::Error(ShellError::TaskError(err)));
                                            }
                                        },
                                        UserStreamMessage::Interrupt => {
                                            //Interrupt
                                            let _ = task_manager.send_message(TaskMessageTx::Terminate);
                                            brutally_terminated = true;
                                            break;
                                        },
                                        UserStreamMessage::Kill => {
                                            //Kill process
                                            if let Err(err) = task_manager.send_message(TaskMessageTx::Kill) {
                                                core.sstream.send(ShellStreamMessage::Error(ShellError::TaskError(err)));
                                            }
                                        },
                                        UserStreamMessage::Signal(signal) => {
                                            //Send signal
                                            if let Err(err) = task_manager.send_message(TaskMessageTx::Signal(signal.clone())) {
                                                core.sstream.send(ShellStreamMessage::Error(ShellError::TaskError(err)));
                                            }
                                        }
                                    }
                                }
                            },
                            Err(_) => {
                                //Terminate task manager
                                let _ = task_manager.send_message(TaskMessageTx::Terminate);
                                break;
                            }
                        }
                        //Sleep for 50ms
                        sleep(Duration::from_millis(50));
                    } //@! End of task manager loop
                    //Get exit code
                    rc = task_manager.join().unwrap_or(255);
                    if brutally_terminated {
                        //Set exit flag to true and break
                        self.exit_flag = Some(rc);
                        break;
                    }
                } else if let Some(func) = chain.function { //@! Functions
                    //Execute function
                    let (exitcode, output): (u8, String) = self.run_expression(core, func.expression);
                    rc = exitcode;
                    //Redirect output
                    if chain.next_relation == TaskRelation::Pipe {
                        //Push output to buffer
                        self.buffer = Some(output);
                    } else {
                        //Redirect output
                        if let Err(err) = self.redirect_function_output(&core.sstream, func.redirection, output) {
                            //Report error
                            if !core.sstream.send(ShellStreamMessage::Error(err)) {
                                break; //Endpoint hung up
                            }
                        }
                    }
                }
            }
            //Set chain to next if possible
            if let Some(next) = chain.next {
                //Always set next to chain
                chain = *next;
                //Verify if relation satisfied
                //If relation is unsatisfied, it will keep iterating, but the block won't be executed
                match chain.next_relation {
                    TaskRelation::And => {
                        //Set next to chain; if exitcode is 0, relation is satisfied
                        if rc == 0 {
                            relation_satisfied = true;
                        } else {
                            relation_satisfied = false;
                        }
                    },
                    TaskRelation::Or => {
                        //If exitcode is successful relation is unsatisfied
                        if rc == 0 {
                            relation_satisfied = false;
                        } else {
                            relation_satisfied = true;
                        }
                    },
                    TaskRelation::Pipe | TaskRelation::Unrelated => {
                        //Relation is always satisfied
                        relation_satisfied = true;
                    }
                }
            } else {
                //Otherwise break
                break;
            }
        } //@! End of loop
        Ok(rc)
    }

    /// ### exec_history
    /// 
    /// Exec a command located in the history
    fn exec_history(&self, core: &mut ShellCore, index: usize) -> Result<u8, ShellError> {
        //Get from history and readline
        match core.history_at(index) {
            Some(cmd) => core.readline(cmd),
            None => Err(ShellError::OutOfHistoryRange)
        }
    }

    /// ### Resolve tasks commands building
    fn chain_task(&self, core: &mut ShellCore, mut head: Task) -> TaskChain {
        let mut chain: Option<TaskChain> = None;
        let mut previous_was_function: bool = false;
        //Iterate over tasks
        loop {
            //Resolve task command
            let mut command: String = head.command[0].clone();
            let mut argv: Vec<String> = Vec::new(); //New argv
            //Check if command is an alias
            if let Some(resolved) = core.alias_get(&command) {
                //Split resolved by space
                for arg in resolved.split_whitespace() {
                    argv.push(String::from(arg));
                }
                //Push head.command[1..] to argv
                for arg in head.command[1..].iter() {
                    argv.push(String::from(arg));
                }
                command = argv[0].clone();
            }
            //Evaluate values
            for arg in argv[1..].iter_mut() {
                //Resolve value
                *arg = self.eval_value(core, arg.to_string());
            }
            //Push argv to task
            head.command = argv;
            //Check if first element is a function
            if let Some(func) = core.function_get(&command) {
                //If it's a function chain a function
                previous_was_function = true;
                match chain.as_mut() {
                    None => {
                        chain = Some(TaskChain::new(None, Some(Function::new(func, head.stdout_redirection.clone())), TaskRelation::Unrelated));
                    },
                    Some(mut chain_obj) => {
                        chain_obj.chain(None, Some(Function::new(func, head.stdout_redirection.clone())), head.relation);
                    }
                };
                //Empty Task.next and relation
                if let Some(task) = head.next.clone() {
                    head.reset_next();
                    //Override head
                    head = *task;
                } else { //No other tasks to iterate through
                    //Break
                    break;
                }
            } else {
                if previous_was_function {
                    previous_was_function = false;
                    //Chain task
                    match chain.as_mut() {
                        None => {
                            chain = Some(TaskChain::new(Some(head.clone()), None, TaskRelation::Unrelated));
                        },
                        Some(mut chain_obj) => {
                            chain_obj.chain(Some(head.clone()), None, head.relation);
                        }
                    }
                }
                //Go ahead
                if let Some(task) = head.next {
                    //Override head
                    head = *task;
                } else { //No other tasks to iterate through
                    //Break
                    break;
                }
            }
        }
        chain.unwrap()
    }

    /// ### exec_function
    /// 
    /// Executes a shell function
    fn exec_function(&mut self, core: &mut ShellCore, function: ShellExpression, argv: Vec<String>) -> (u8, String) {
        //Argv[0] => function name, [1..] => arguments
        //Set arguments to storage
        for (index, arg) in argv.iter().enumerate() {
            core.storage_set(index.to_string(), arg.clone());
        }
        //Execute function
        let (rc, output): (u8, String) = self.run_expression(core, function);
        //Unset argument from storage
        for (index, arg) in argv.iter().enumerate() {
            core.value_unset(&index.to_string());
        }
        //Return rc and output
        (rc, output)
    }

    /// ### exit
    /// 
    /// Terminates Expression execution and shell
    fn exit(&mut self, core: &mut ShellCore, exit_code: u8) {
        //Exit
        self.exit_flag = Some(exit_code);
        core.exit();
    }

    /// ### export
    /// 
    /// Export a variable in the environment
    fn export(&mut self, core: &mut ShellCore, key: String, value: ShellExpression) {
        let (_, value): (u8, String) = self.run_expression(core, value);
        core.environ_set(key, value);
    }

    /// ### foreach
    /// 
    /// Perform a for statement
    fn foreach(&mut self, core: &mut ShellCore, key: String, condition: ShellExpression, expression: ShellExpression) {
        //Get result of condition
        let (rc, output): (u8, String) = self.run_expression(core, condition);
        if rc != 0 {
            return;
        }
        //Iterate over output split by whitespace
        for i in output.split_whitespace() {
            //Export key to storage
            core.storage_set(key.clone(), i.to_string());
            //Execute expression
            let _ = self.run_expression(core, expression.clone());
        }
        //Remove key from storage
        core.value_unset(&key);
    }

    /// ### ifcond
    /// 
    /// Perform if statement
    fn ifcond(&mut self, core: &mut ShellCore, condition: ShellExpression, if_perform: ShellExpression, else_perform: Option<ShellExpression>) {
        //Get result of condition
        let (rc, _): (u8, String) = self.run_expression(core, condition);
        //If rc is 0 => execute if perform
        if rc == 0 {
            //Execute expression
            let _ = self.run_expression(core, if_perform);
        } else if let Some(else_perform) = else_perform {
            //Perform else if set
            let _ = self.run_expression(core, else_perform);
        }
    }

    //TODO: let statement

    /// ### popd_back
    /// 
    /// Execute popd_back statement. Returns the popped directory if exists
    fn popd_back(&self, core: &mut ShellCore) -> Option<PathBuf> {
        core.popd_back()
    }

    /// ### popd_back
    /// 
    /// Execute popd_front statement. Returns the popped directory if exists
    fn popd_front(&self, core: &mut ShellCore) -> Option<PathBuf> {
        core.popd_front()
    }

    /// ### pushd
    /// 
    /// Execute pushd statement.
    fn pushd(&self, core: &mut ShellCore, dir: PathBuf) {
        core.pushd(dir);
    }

    /// ### read
    /// 
    /// Execute read statement, which means it waits for input until arrives; if the input has a maximum size, it gets cut to the maximum size
    /// The data read is exported to result_key or to REPLY if not provided
    fn read(&mut self, core: &mut ShellCore, prompt: Option<String>, max_size: Option<usize>, result_key: Option<String>) {
        let prompt: String = match prompt {
            Some(p) => p,
            None => String::new()
        };
        //Send prompt as output
        let _ = core.sstream.send(ShellStreamMessage::Output((Some(prompt), None)));
        //Define the key name
        let key: String = match result_key {
            Some(k) => k,
            None => String::from("REPLY")
        };
        //Read
        loop {
            //Try to read from sstream
            match core.sstream.receive() {
                Ok(inbox) => {
                    //Iterate over inbox
                    for message in inbox.iter() {
                        match message {
                            UserStreamMessage::Input(input) => { //If input return input or 
                                match max_size {
                                    None => {
                                        //Export variable to storage
                                        core.storage_set(key, input.clone());
                                        return;
                                    },
                                    Some(size) => {
                                        let value: String = String::from(&input[..size]);
                                        core.storage_set(key, value);
                                        return;
                                    }
                                }
                            },
                            UserStreamMessage::Kill => return,
                            UserStreamMessage::Signal(_) => return,
                            UserStreamMessage::Interrupt => {
                                self.exit_flag = Some(255);
                                return;
                            }
                        }
                    }
                },
                Err(_) => {
                    self.exit_flag = Some(255);
                    return;
                }
            }
        }
    }

    /// ### set
    /// 
    /// Set a key with its associated value in the Shell session storage
    fn set(&mut self, core: &mut ShellCore, key: String, value: ShellExpression) {
        let (_, value): (u8, String) = self.run_expression(core, value);
        core.storage_set(key, value);
    }

    /// ### source
    /// 
    /// Source file
    fn source(&self, core: &mut ShellCore, file: PathBuf) -> bool {
        //Source file, report any error
        if let Err(err) = core.source(file) {
            //Report error
            core.sstream.send(ShellStreamMessage::Error(err));
            false
        } else {
            true
        }
    }

    //TODO: time (set instant, execute command, get duration, return duration)

    /// ### eval_value
    /// 
    /// Evaluate value
    fn eval_value(&self, core: &mut ShellCore, value: String) -> String {
        //Treat variables
        let mut outval: String = value.clone();
        if value.starts_with("${") {
            //Get value from core
            let last_pos_index: usize = value.len() - 1;
            let value: String = String::from(&value[2..last_pos_index]);
            outval = match core.value_get(&value) {
                Some(val) => val,
                None => String::from("")
            };
        } else if value.starts_with("$") {
            //Get value from core
            let value: String = String::from(&value[1..]);
            outval = match core.value_get(&value) {
                Some(val) => val,
                None => String::from("")
            };
        }
        //Once out of variable control, let's look for wildcards
        if (outval.matches("*").count() > 0 && outval.matches("*").count() != outval.matches("\\*").count()) || (outval.matches("?").count() > 0 && outval.matches("?").count() != outval.matches("\\?").count()) {
            //Resolve wildcards, we expect value to be a path. In case of wild cards, value is a string with matched files separated by whitespace
            let path: &Path = Path::new(outval.as_str());
            //If path is relative, get absolute path
            let abs_path: PathBuf = match path.is_relative() {
                true => {
                    let mut abs_path: PathBuf = core.get_wrkdir();
                    abs_path.push(path);
                    abs_path
                },
                false => PathBuf::from(path)
            };
            //Get files in path
            let mut result: String = String::new();
            if let Ok(records) = glob(abs_path.to_str().unwrap()) {
                for entry in records {
                    if let Ok(path) = entry {
                        result.push_str(format!("{} ", path.display()).as_str());
                    }
                }
            }
            result
        } else {
            //Else return value
            outval
        }
    }

    /// ### while_loop
    /// 
    /// Perform While shell statement
    fn while_loop(&mut self, core: &mut ShellCore, condition: ShellExpression, expression: ShellExpression) {
        loop {
            let (rc, _): (u8, String) = self.run_expression(core, condition.clone());
            if rc != 0 { //If rc is NOT 0, break
                break;
            }
            //Otherwise perform expression
            let _ = self.run_expression(core, expression.clone());
        }
    }

    /// ### get_expression_str_value
    /// 
    /// Return the string output and the result of an expression.
    /// This function is very important since must be used by all the other statements which uses an expression (e.g. set, export, case, if...)
    fn run_expression(&mut self, core: &mut ShellCore, expression: ShellExpression) -> (u8, String) {
        //TODO: implement
        let mut rc: u8 = 0;
        let mut output: String = String::new();
        //Iterate over expression
        //NOTE: the expression is executed as long as it's possible
        for statement in expression.statements.iter() {
            //Match statement and execute it
            //TODO: check exit flag
            //TODO: look for inputs
        }
        (rc, output)
    }

    /// ### redirect_function_output
    ///
    /// Handle output redirections in a single method
    fn redirect_function_output(&self, sstream: &ShellStream, redirection: Redirection, output: String) -> Result<(), ShellError> {
        match redirection {
            Redirection::Stdout => {
                //Send output
                sstream.send(ShellStreamMessage::Output((Some(output), None)));
            },
            Redirection::Stderr => {
                sstream.send(ShellStreamMessage::Output((None, Some(output))));
            }
            Redirection::File(file, file_mode) => {
                match OpenOptions::new().create(true).append(file_mode == FileRedirectionType::Append).truncate(file_mode == FileRedirectionType::Truncate).open(file.as_str()) {
                    Ok(mut f) => {
                        if let Err(e) = write!(f, "{}", output) {
                            return Err(ShellError::TaskError(TaskError::new(TaskErrorCode::IoError,format!("Could not write to file {}: {}", file, e))))
                        } else {
                            return Ok(())
                        }
                    }
                    Err(e) => return Err(ShellError::TaskError(TaskError::new(TaskErrorCode::IoError,format!("Could not open file {}: {}", file, e)))),
                }
            }
        }
        Ok(())
    }
}

impl TaskChain {

    /// ### new
    /// 
    /// Instantiates a new TaskChain. This must be called for the first element only
    pub(self) fn new(task: Option<Task>, function: Option<Function>, prev_relation: TaskRelation) -> TaskChain {
        TaskChain {
            task: task,
            function: function,
            prev_relation: prev_relation,
            next_relation: TaskRelation::Unrelated,
            next: None
        }
    }

    /// ### chain
    /// 
    /// Chain a Task to the back current one
    pub(self) fn chain(&mut self, next_task: Option<Task>, next_function: Option<Function>, relation: TaskRelation) {
        //If next is None, set Next as new Task, otherwise pass new task to the next of the next etc...
        match &mut self.next {
            None => self.next = {
                //Set current relation to relation
                self.next_relation = relation;
                Some(Box::new(TaskChain::new(next_task, next_function, self.next_relation)))
            },
            Some(next) => next.chain(next_task, next_function, relation)
        }
    }
}

impl Function {

    /// ### new
    /// 
    /// Instantiate a new Function
    pub(self) fn new(expression: ShellExpression, redirection: Redirection) -> Function {
        Function {
            expression: expression,
            redirection: redirection
        }
    }
}

//@! Tests

#[cfg(test)]
mod tests {

    use super::*;
    use crate::parsers::bash::Bash;
    use crate::ShellStatement;
    use crate::UserStream;

    use std::mem::discriminant;

    #[test]
    fn test_runner_new() {
        let runner: ShellRunner = ShellRunner::new();
        assert!(runner.buffer.is_none());
        assert!(runner.exit_flag.is_none());
    }

    #[test]
    fn test_runner_eval_values() {
        let runner: ShellRunner = ShellRunner::new();
        //Instantiate cores
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(Some(PathBuf::from("/bin/")), 128, Box::new(Bash {}));
        //Set test values into storage
        core.storage_set(String::from("KEYTEST1"), String::from("BAR"));
        core.storage_set(String::from("KEYTEST2"), String::from("/*"));
        core.storage_set(String::from("KEYTEST3"), String::from("./*"));
        //Evaluate values
        assert_eq!(runner.eval_value(&mut core, String::from("$NOKEY")), String::from(""));
        assert_eq!(runner.eval_value(&mut core, String::from("${NOKEY}")), String::from(""));
        assert_eq!(runner.eval_value(&mut core, String::from("$KEYTEST1")), String::from("BAR"));
        assert_eq!(runner.eval_value(&mut core, String::from("${KEYTEST1}")), String::from("BAR"));
        assert_ne!(runner.eval_value(&mut core, String::from("${KEYTEST2}")), String::from("/*"));
        assert!(runner.eval_value(&mut core, String::from("${KEYTEST2}")).matches("/bin").count() > 0);
        assert_ne!(runner.eval_value(&mut core, String::from("${KEYTEST3}")), String::from("./*"));
        assert_ne!(runner.eval_value(&mut core, String::from("${KEYTEST3}")), String::from(""));
        assert!(runner.eval_value(&mut core, String::from("${KEYTEST3}")).matches("cp").count() > 0);
    }

    #[test]
    fn test_runner_function() {
        //Instantiate an expression
        let statements: Vec<ShellStatement> = vec![ShellStatement::Exit(0)];
        let expression: ShellExpression = ShellExpression {
            statements: statements
        };
        //Instantiate function
        let function: Function = Function::new(expression, Redirection::Stdout);
        assert_eq!(function.redirection, Redirection::Stdout);
        assert_eq!(function.expression.statements.len(), 1);
        assert_eq!(discriminant(&function.expression.statements[0]), discriminant(&ShellStatement::Exit(0)));
    }

    #[test]
    fn test_runner_chain() {
        //Instantiate an expression
        let statements: Vec<ShellStatement> = vec![ShellStatement::Exit(0)];
        let expression: ShellExpression = ShellExpression {
            statements: statements
        };
        //Instantiate function
        let function: Function = Function::new(expression, Redirection::Stdout);
        let mut chain: TaskChain = TaskChain::new(None, Some(function), TaskRelation::Unrelated);
        //Verify constructor
        assert_eq!(chain.prev_relation, TaskRelation::Unrelated);
        assert_eq!(chain.next_relation, TaskRelation::Unrelated);
        assert!(chain.next.is_none());
        assert!(chain.function.is_some());
        assert!(chain.task.is_none());
        //Prepare stuff to chain a new object
        let expression: ShellExpression = ShellExpression {
            statements: vec![ShellStatement::Value(String::from("BAR"))]
        };
        let statements: Vec<ShellStatement> = vec![ShellStatement::Set(String::from("FOO"), expression)];
        let expression: ShellExpression = ShellExpression {
            statements: statements
        };
        let function: Function = Function::new(expression, Redirection::Stdout);
        //Chain a new function
        chain.chain(None, Some(function), TaskRelation::And);
        assert_eq!(chain.next_relation, TaskRelation::And);
        assert_eq!(chain.prev_relation, TaskRelation::Unrelated);
        assert!(chain.next.is_some());
        //Verify next
        let next: &TaskChain = chain.next.as_ref().unwrap();
        assert!(next.function.is_some());
        assert!(next.task.is_none());
        assert_eq!(next.prev_relation, TaskRelation::And);
        assert_eq!(next.next_relation, TaskRelation::Unrelated);
        assert!(next.next.is_none());
        //Prepare to chain a 3rd element
        let statements: Vec<ShellStatement> = vec![ShellStatement::Read(None, None, None)];
        let expression: ShellExpression = ShellExpression {
            statements: statements
        };
        let function: Function = Function::new(expression, Redirection::Stdout);
        //Chain a 3rd element
        chain.chain(None, Some(function), TaskRelation::Or);
        let next: &TaskChain = chain.next.as_ref().unwrap();
        //Check if the relation between the 1st and the 2nd has been preserved
        assert_eq!(chain.next_relation, TaskRelation::And);
        assert_eq!(chain.prev_relation, TaskRelation::Unrelated);
        assert!(chain.next.is_some());
        //Okay, now verify the relation between the 2nd and the 3rd
        assert_eq!(next.prev_relation, TaskRelation::And);
        assert_eq!(next.next_relation, TaskRelation::Or); //NOTE: this was Unrelated before
        assert!(next.next.is_some()); //Now is some
        let next: &TaskChain = next.next.as_ref().unwrap();
        //Verify 3rd block
        assert!(next.function.is_some());
        assert!(next.task.is_none());
        assert_eq!(next.prev_relation, TaskRelation::Or);
        assert_eq!(next.next_relation, TaskRelation::Unrelated);
        assert!(next.next.is_none());
    }

}
