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

use crate::{FileRedirectionType, MathError, MathOperator, Redirection};
use crate::{ShellCore, ShellError, ShellExpression, ShellRunner, ShellStatement, ShellStream, ShellStreamMessage};
use crate::{TaskManager, Task, UserStreamMessage};
use crate::tasks::{TaskError, TaskErrorCode, TaskMessageRx, TaskMessageTx, TaskRelation};

use glob::glob;
use std::collections::{HashMap, VecDeque};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;
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
    fn alias(&mut self, core: &mut ShellCore, name: Option<String>, command: Option<String>) -> u8 {
        if name.is_some() && command.is_some() {
            match core.alias_set(name.unwrap(), command.unwrap()) {
                true => 0,
                false => 1
            }
        } else if name.is_some() && command.is_none() {
            //Send alias value
            match core.alias_get(name.as_ref().unwrap()) {
                Some(cmd) => {
                    let mut alias_list: HashMap<String, String> = HashMap::new();
                    alias_list.insert(name.unwrap().clone(), cmd);
                    //Send alias list
                    if ! core.sstream.send(ShellStreamMessage::Alias(alias_list)) {
                        self.exit_flag = Some(255);
                    }
                    0
                },
                None => {
                    //Send err alias
                    if ! core.sstream.send(ShellStreamMessage::Error(ShellError::NoSuchAlias(name.unwrap().clone()))) {
                        self.exit_flag = Some(255);
                    }
                    1
                }
            }
        } else if name.is_none() && command.is_none() {
            //Return all alias
            let alias_list: HashMap<String, String> = core.alias_get_all();
            //Send alias list
            if ! core.sstream.send(ShellStreamMessage::Alias(alias_list)) {
                self.exit_flag = Some(255);
            }
            0
        } else {
            1
        }
    }

    /// ### unalias
    /// 
    /// Remove an alias from core
    fn unalias(&mut self, core: &mut ShellCore, name: String) -> u8 {
        match core.unalias(&name) {
            Some(_) => {
                0
            },
            None => {
                //Send err alias
                if ! core.sstream.send(ShellStreamMessage::Error(ShellError::NoSuchAlias(name))) {
                    self.exit_flag = Some(255);
                }
                1
            }
        }
    }

    /// ### case
    /// 
    /// Perform case statement
    fn case(&mut self, core: &mut ShellCore, what: ShellExpression, cases: Vec<(ShellExpression, ShellExpression)>) -> u8 {
        let mut exitcode: u8 = 0;
        let (_, output): (u8, String) = self.run_expression(core, what);
        //Output in
        for case in cases.iter() {
            let (_, case_match): (u8, String) = self.run_expression(core, case.0.clone());
            //If case match is equal to output, execute case perform
            if case_match == output || case_match == "\\*" {
                let (rc, _): (u8, String) = self.run_expression(core, case.1.clone());
                exitcode = rc;
                break; //Stop iterating
            }
        }
        exitcode
    }

    /// ### change_directory
    /// 
    /// Execute cd statement
    fn change_directory(&mut self, core: &mut ShellCore, path: PathBuf) -> u8 {
        if let Err(err) = core.change_directory(path) {
            //Send error
            if !core.sstream.send(ShellStreamMessage::Error(err)) {
                //Set exit flag
                self.exit_flag = Some(255);
            }
            1
        } else {
            0
        }
    }

    /// ### dirs
    /// 
    /// Sends the directories in the core stack
    fn dirs(&mut self, core: &mut ShellCore) -> u8 {
        let dirs: VecDeque<PathBuf> = core.dirs();
        if ! core.sstream.send(ShellStreamMessage::Dirs(dirs)) {
            //Set exit flag
            self.exit_flag = Some(255);
        }
        0
    }

    /// ### exec
    /// 
    /// Executes through the task manager a Task
    fn exec(&mut self, core: &mut ShellCore, task: Task) -> u8 {
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
        rc
    }

    /// ### exec_history
    /// 
    /// Exec a command located in the history
    fn exec_history(&mut self, core: &mut ShellCore, index: usize) -> u8 {
        //Get from history and readline
        match core.history_at(index) {
            Some(cmd) => match core.readline(cmd) {
                Ok(rc) => rc,
                Err(err) => {
                    //Report error
                    if !core.sstream.send(ShellStreamMessage::Error(err)) {
                        self.exit_flag = Some(255);
                    }
                    255
                }
            }
            None => {
                if !core.sstream.send(ShellStreamMessage::Error(ShellError::OutOfHistoryRange)) {
                    self.exit_flag = Some(255);
                }
                255
            }
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

    /// ### exec_time
    /// 
    /// Executes a command with duration
    fn exec_time(&mut self, core: &mut ShellCore, task: Task) -> u8 {
        let t_start: Instant = Instant::now();
        let rc: u8 = self.exec(core, task);
        //Report execution time
        let exec_time: Duration = t_start.elapsed();
        //Report execution time
        if ! core.sstream.send(ShellStreamMessage::Time(exec_time)) {
            //Set exit flag
            self.exit_flag = Some(255);
        }
        rc
    }

    /// ### exit
    /// 
    /// Terminates Expression execution and shell
    fn exit(&mut self, core: &mut ShellCore, exit_code: u8) {
        //Exit
        self.exit_flag = Some(exit_code);
    }

    /// ### export
    /// 
    /// Export a variable in the environment
    fn export(&mut self, core: &mut ShellCore, key: String, value: ShellExpression) -> u8 {
        let (_, value): (u8, String) = self.run_expression(core, value);
        match core.environ_set(key.clone(), value) {
            true => 0,
            false => {
                //Report error
                if ! core.sstream.send(ShellStreamMessage::Error(ShellError::BadValue(key))) {
                    //Set exit flag
                    self.exit_flag = Some(255);
                }
                1
            }
        }
    }

    /// ### foreach
    /// 
    /// Perform a for statement
    fn foreach(&mut self, core: &mut ShellCore, key: String, condition: ShellExpression, expression: ShellExpression) -> u8 {
        //Get result of condition
        let mut exitcode: u8 = 0;
        let (rc, output): (u8, String) = self.run_expression(core, condition);
        if rc != 0 {
            return 255;
        }
        //Iterate over output split by whitespace
        for i in output.split_whitespace() {
            //Export key to storage
            core.storage_set(key.clone(), i.to_string());
            //Execute expression
            let (rc, _): (u8, String) = self.run_expression(core, expression.clone());
            exitcode = rc;
        }
        //Remove key from storage
        core.value_unset(&key);
        exitcode
    }

    /// ### function
    /// 
    /// Add a new function to core
    fn function(&mut self, core: &mut ShellCore, name: String, expression: ShellExpression) -> u8 {
        match core.function_set(name, expression) {
            true => 0,
            false => 1
        }
    }

    /// ### ifcond
    /// 
    /// Perform if statement
    fn ifcond(&mut self, core: &mut ShellCore, condition: ShellExpression, if_perform: ShellExpression, else_perform: Option<ShellExpression>) -> u8 {
        //Get result of condition
        let mut exitcode: u8 = 0;
        let (rc, _): (u8, String) = self.run_expression(core, condition);
        //If rc is 0 => execute if perform
        if rc == 0 {
            //Execute expression
            let (rc, _) = self.run_expression(core, if_perform);
            exitcode = rc;
        } else if let Some(else_perform) = else_perform {
            //Perform else if set
            let (rc, _) = self.run_expression(core, else_perform);
            exitcode = rc;
        }
        exitcode
    }

    /// ### let_perform
    /// 
    /// Perform let statement
    fn let_perform(&mut self, core: &mut ShellCore, dest: String, operator1: ShellExpression, operation: MathOperator, operator2: ShellExpression) -> u8 {        
        //Get output for operator 1
        let (_, output): (u8, String) = self.run_expression(core, operator1);
        //Try to convert output to number
        let operator1: isize = output.parse::<isize>().unwrap_or(0);
        //Get output for operator 2
        let (_, output): (u8, String) = self.run_expression(core, operator2);
        //Try to convert output to number
        let operator2: isize = output.parse::<isize>().unwrap_or(0);
        //Perform math expression
        let result: isize = match operation {
            MathOperator::And => {
                operator1 & operator2
            },
            MathOperator::Divide => {
                if operator2 == 0 { //Report error if dividing by 0
                    if ! core.sstream.send(ShellStreamMessage::Error(ShellError::Math(MathError::DividedByZero))) {
                        //Set exit flag
                        self.exit_flag = Some(255);
                    }
                    return 1
                } else {
                    operator1 / operator2
                }
            },
            MathOperator::Equal => {
                (operator1 == operator2) as isize
            },
            MathOperator::Module => {
                if operator2 == 0 { //Report error if dividing by 0
                    if ! core.sstream.send(ShellStreamMessage::Error(ShellError::Math(MathError::DividedByZero))) {
                        //Set exit flag
                        self.exit_flag = Some(255);
                    }
                    return 1
                } else {
                    operator1 % operator2
                }
            },
            MathOperator::Multiply => {
                operator1 * operator2
            },
            MathOperator::NotEqual => {
                (operator1 != operator2) as isize
            },
            MathOperator::Or => {
                operator1 | operator2
            },
            MathOperator::Power => {
                if operator2 < 0 {
                    if ! core.sstream.send(ShellStreamMessage::Error(ShellError::Math(MathError::NegativePower))) {
                        //Set exit flag
                        self.exit_flag = Some(255);
                    }
                }
                operator1.pow(operator2 as u32)
            },
            MathOperator::ShiftLeft => {
                operator1 >> operator2
            },
            MathOperator::ShiftRight => {
                operator1 << operator2
            },
            MathOperator::Subtract => {
                operator1 - operator2
            },
            MathOperator::Sum => {
                operator1 + operator2
            },
            MathOperator::Xor => {
                operator1 ^ operator2
            }
        };
        //Store variable if possible
        match core.storage_set(dest, result.to_string()) {
            true => 0,
            false => 1
        }
    }

    /// ### popd_back
    /// 
    /// Execute popd_back statement. Returns the popped directory if exists
    fn popd_back(&mut self, core: &mut ShellCore) -> u8 {
        if let Some(dir) = core.popd_back() {
            let mut dirs: VecDeque<PathBuf> = VecDeque::with_capacity(1);
            dirs.push_back(dir);
            if ! core.sstream.send(ShellStreamMessage::Dirs(dirs)) {
                //Set exit flag
                self.exit_flag = Some(255);
            }
            0
        } else {
            1
        }
    }

    /// ### popd_back
    /// 
    /// Execute popd_front statement. Returns the popped directory if exists
    fn popd_front(&mut self, core: &mut ShellCore) -> u8 {
        if let Some(dir) = core.popd_front() {
            let mut dirs: VecDeque<PathBuf> = VecDeque::with_capacity(1);
            dirs.push_back(dir);
            if ! core.sstream.send(ShellStreamMessage::Dirs(dirs)) {
                //Set exit flag
                self.exit_flag = Some(255);
            }
            0
        } else {
            1
        }
    }

    /// ### pushd
    /// 
    /// Execute pushd statement.
    fn pushd(&mut self, core: &mut ShellCore, dir: PathBuf) -> u8 {
        core.pushd(dir);
        //Returns dir
        self.dirs(core);
        0
    }

    /// ### read
    /// 
    /// Execute read statement, which means it waits for input until arrives; if the input has a maximum size, it gets cut to the maximum size
    /// The data read is exported to result_key or to REPLY if not provided
    fn read(&mut self, core: &mut ShellCore, prompt: Option<String>, max_size: Option<usize>, result_key: Option<String>) -> u8 {
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
                                        match core.storage_set(key, input.clone()) {
                                            true => return 0,
                                            false => return 1
                                        }
                                    },
                                    Some(size) => {
                                        let value: String = String::from(&input[..size]);
                                        match core.storage_set(key, value) {
                                            true => return 0,
                                            false => return 1
                                        }
                                    }
                                }
                            },
                            UserStreamMessage::Kill => return 1,
                            UserStreamMessage::Signal(_) => return 1,
                            UserStreamMessage::Interrupt => {
                                self.exit_flag = Some(255);
                                return 1
                            }
                        }
                    }
                },
                Err(_) => {
                    self.exit_flag = Some(255);
                    return 1
                }
            }
        }
    }

    /// ### set
    /// 
    /// Set a key with its associated value in the Shell session storage
    fn set(&mut self, core: &mut ShellCore, key: String, value: ShellExpression) -> u8 {
        let (_, value): (u8, String) = self.run_expression(core, value);
        match core.storage_set(key.clone(), value) {
            true => 0,
            false => {
                //Report error
                if ! core.sstream.send(ShellStreamMessage::Error(ShellError::BadValue(key))) {
                    //Set exit flag
                    self.exit_flag = Some(255);
                }
                1
            }
        }
    }

    /// ### source
    /// 
    /// Source file
    fn source(&self, core: &mut ShellCore, file: PathBuf) -> u8 {
        //Source file, report any error
        if let Err(err) = core.source(file) {
            //Report error
            core.sstream.send(ShellStreamMessage::Error(err));
            0
        } else {
            1
        }
    }

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
    fn while_loop(&mut self, core: &mut ShellCore, condition: ShellExpression, expression: ShellExpression) -> u8 {
        let mut exitcode: u8 = 0;
        loop {
            let (rc, _): (u8, String) = self.run_expression(core, condition.clone());
            if rc != 0 { //If rc is NOT 0, break
                break;
            }
            //Otherwise perform expression
            let (rc, _) = self.run_expression(core, expression.clone());
            exitcode = rc;
        }
        exitcode
    }

    /// ### get_expression_str_value
    /// 
    /// Return the string output and the result of an expression.
    /// This function is very important since must be used by all the other statements which uses an expression (e.g. set, export, case, if...)
    fn run_expression(&mut self, core: &mut ShellCore, expression: ShellExpression) -> (u8, String) {
        let mut rc: u8 = 0;
        let mut output: String = String::new();
        //Iterate over expression
        //NOTE: the expression is executed as long as it's possible
        for statement in expression.statements.iter() {
            //Match statement and execute it
            match statement {
                ShellStatement::Alias(name, cmd) => {
                    rc = self.alias(core, name.clone(), cmd.clone());
                },
                ShellStatement::Break => {
                    break; //Stop iterating
                },
                ShellStatement::Case(what, cases) => {
                    rc = self.case(core, what.clone(), cases.clone());
                },
                ShellStatement::Cd(path) => {
                    rc = self.change_directory(core, path.clone());
                },
                ShellStatement::Continue => {
                    //Keep iterating
                    continue;
                },
                ShellStatement::Dirs => {
                    rc = self.dirs(core);
                },
                ShellStatement::Exec(task) => {
                    rc = self.exec(core, task.clone());
                },
                ShellStatement::ExecHistory(index)  => {
                    rc = self.exec_history(core, *index);
                },
                ShellStatement::Exit(exitcode) => {
                    self.exit(core, *exitcode);
                },
                ShellStatement::Export(key, value) => {
                    rc = self.export(core, key.clone(), value.clone());
                },
                ShellStatement::For(what, when, perform) => {
                    rc = self.foreach(core, what.clone(), when.clone(), perform.clone());
                },
                ShellStatement::Function(name, expression) => {
                    rc = self.function(core, name.clone(), expression.clone());
                },
                ShellStatement::If(what, perform_if, perform_else) => {
                    rc = self.ifcond(core, what.clone(), perform_if.clone(), perform_else.clone());
                },
                ShellStatement::Let(dest, operator1, operation, operator2) => {
                    rc = self.let_perform(core, dest.clone(), operator1.clone(), operation.clone(), operator2.clone());
                },
                ShellStatement::PopdBack => {
                    rc = self.popd_back(core);
                },
                ShellStatement::PopdFront => {
                    rc = self.popd_front(core);
                },
                ShellStatement::Pushd(dir) => {
                    rc = self.pushd(core, dir.clone());
                },
                ShellStatement::Read(prompt, length, result_key) => {
                    rc = self.read(core, prompt.clone(), length.clone(), result_key.clone());
                },
                ShellStatement::Return(ret) => {
                    return (*ret, output);
                },
                ShellStatement::Set(key, value) => {
                    rc = self.set(core, key.clone(), value.clone());
                },
                ShellStatement::Source(file) => {
                    rc = self.source(core, file.clone());
                },
                ShellStatement::Time(task) => {
                    rc = self.exec_time(core, task.clone());
                },
                ShellStatement::Unalias(alias) => {
                    rc = self.unalias(core, alias.clone());
                },
                ShellStatement::Value(val) => {
                    output = self.eval_value(core, val.clone());
                    rc = 0;
                },
                ShellStatement::While(until, perform) => {
                    rc = self.while_loop(core, until.clone(), perform.clone());
                }
            }
            //look for inputs
            match core.sstream.receive() {
                Ok(inbox) => {
                    //Iterate over received messages
                    for message in inbox.iter() {
                        //Match message and handle it
                        match message {
                            UserStreamMessage::Input(stdin) => {
                                //Store input into runner buffer
                                self.buffer = match self.buffer.as_mut() {
                                    Some(buf) => {
                                        buf.push_str(stdin.as_str());
                                        Some(buf.to_string())
                                    },
                                    None => Some(stdin.to_string())
                                };
                            },
                            UserStreamMessage::Interrupt => {
                                self.exit_flag = Some(255);
                            },
                            UserStreamMessage::Kill => {
                                self.exit_flag = Some(9);
                            },
                            UserStreamMessage::Signal(sig) => {
                                self.exit_flag = Some(*sig as u8);
                            }
                        }
                    }
                },
                Err(_) => {
                    //Endpoint hung up, terminate
                    self.exit_flag = Some(255);
                }
            }
            //check exit flag
            if let Some(exitcode) = self.exit_flag {
                //If exit flag is set, terminate expression execution
                rc = exitcode;
                break;
            }
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
    use crate::UnixSignal;

    use std::mem::discriminant;
    use std::mem::drop;

    #[test]
    fn test_runner_new() {
        let runner: ShellRunner = ShellRunner::new();
        assert!(runner.buffer.is_none());
        assert!(runner.exit_flag.is_none());
    }

    #[test]
    fn test_runner_alias() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Verify alias doesn't exist
        assert!(core.alias_get(&String::from("ll")).is_none());
        //Set alias
        assert_eq!(runner.alias(&mut core, Some(String::from("ll")), Some(String::from("ls -l"))), 0);
        assert!(core.alias_get(&String::from("ll")).is_some());
        //Let's get that alias
        assert_eq!(runner.alias(&mut core, Some(String::from("ll")), None), 0);
        //We should have received the alias in the ustream
        if let ShellStreamMessage::Alias(alias) = &ustream.receive().unwrap()[0] {
            assert_eq!(*alias.get(&String::from("ll")).unwrap(), String::from("ls -l"));
        } else {
            panic!("Not an Alias");
        }
        //Let's get an alias which doesn't exist
        assert_eq!(runner.alias(&mut core, Some(String::from("foobar")), None), 1);
        if let ShellStreamMessage::Error(err) = &ustream.receive().unwrap()[0] {
            assert_eq!(discriminant(err), discriminant(&ShellError::NoSuchAlias(String::from("foobar"))));
        } else {
            panic!("Not an error");
        }
        //Let's insert another alias
        assert_eq!(runner.alias(&mut core, Some(String::from("dirsize")), Some(String::from("du -hs"))), 0);
        //Let's get all aliases
        assert_eq!(runner.alias(&mut core, None, None), 0);
        if let ShellStreamMessage::Alias(alias) = &ustream.receive().unwrap()[0] {
            assert_eq!(*alias.get(&String::from("ll")).unwrap(), String::from("ls -l"));
            assert_eq!(*alias.get(&String::from("dirsize")).unwrap(), String::from("du -hs"));
        } else {
            panic!("Not an alias");
        }
        assert_eq!(runner.alias(&mut core, Some(String::from("grep")), Some(String::from("grep --color=auto"))), 0);
        assert_eq!(runner.unalias(&mut core, String::from("grep")), 0);
        //Try to unalias something that doesn't exist
        assert_eq!(runner.unalias(&mut core, String::from("fooooooo")), 1);
        if let ShellStreamMessage::Error(err) = &ustream.receive().unwrap()[0] {
            assert_eq!(discriminant(err), discriminant(&ShellError::NoSuchAlias(String::from("foobar"))));
        } else {
            panic!("Not an error");
        }
        //Drop ustream and fail alias get
        drop(ustream);
        assert_eq!(runner.alias(&mut core, Some(String::from("ll")), None), 0);
        assert!(runner.exit_flag.is_some());
        //Reset
        runner.exit_flag = None;
        runner.alias(&mut core, None, None);
        assert!(runner.exit_flag.is_some());
        //Reset
        runner.exit_flag = None;
        runner.alias(&mut core, Some(String::from("ll")), Some(String::from("ls -l --color=auto")));
        //Verify alias changed actually
        assert_eq!(core.alias_get(&String::from("ll")).unwrap(), String::from("ls -l --color=auto"));
        //Try to set a bad alias
        assert_eq!(runner.alias(&mut core, Some(String::from("l/l")), Some(String::from("ls -l"))), 1);
        //Try to remove alias
    }

    #[test]
    fn test_runner_change_directory() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        assert_eq!(runner.change_directory(&mut core, PathBuf::from("/tmp/")), 0);
        assert_eq!(core.get_wrkdir(), PathBuf::from("/tmp/"));
        //Try to change directory to not existing path
        assert_eq!(runner.change_directory(&mut core, PathBuf::from("/onett/")), 1);
        //Directory shouldn't have changed
        assert_eq!(core.get_wrkdir(), PathBuf::from("/tmp/"));
        //Verify we received an error
        if let ShellStreamMessage::Error(err) = &ustream.receive().unwrap()[0] {
            assert_eq!(discriminant(err), discriminant(&ShellError::NoSuchFileOrDirectory(PathBuf::from("/onett/"))));
        } else {
            panic!("Not an error");
        }
        //Drop ustream and change directory
        drop(ustream);
        assert_eq!(runner.change_directory(&mut core, PathBuf::from("/onett/")), 1);
        assert!(runner.exit_flag.is_some());
    }

    //TODO: chain task
    //TODO: exec
    //TODO: exec_history
    //TODO: exec_function
    //TODO: exec_time

    #[test]
    fn test_runner_exit() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        runner.exit(&mut core, 0);
        assert_eq!(runner.exit_flag.unwrap(), 0);
    }

    #[test]
    fn test_runner_export() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //TODO: implement (requires run_expression)
    }
    
    //TODO: foreach
    //TODO: ifcond
    //TODO: let

    #[test]
    fn test_runner_dirs() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Get dirs when empty
        assert_eq!(runner.dirs(&mut core), 0);
        if let ShellStreamMessage::Dirs(dirs) = &ustream.receive().unwrap()[0] {
            assert_eq!(dirs.len(), 1); //Contains home
        } else {
            panic!("Not a dirs");
        }
        //Push directory
        assert_eq!(runner.pushd(&mut core, PathBuf::from("/tmp/")), 0);
        if let ShellStreamMessage::Dirs(dirs) = &ustream.receive().unwrap()[0] {
            assert_eq!(dirs.len(), 2); //Contains home and tmp
        } else {
            panic!("Not a dirs");
        }
        //Popd
        assert_eq!(runner.popd_back(&mut core), 0);
        if let ShellStreamMessage::Dirs(dirs) = &ustream.receive().unwrap()[0] {
            assert_eq!(dirs.len(), 1); //Contains home and tmp
            assert_eq!(*dirs[0], core.get_home());
        } else {
            panic!("Not a dirs");
        }
        //You can't empty directory stack, so 1 will be returned
        assert_eq!(runner.popd_front(&mut core),1);
        runner.dirs(&mut core);
        if let ShellStreamMessage::Dirs(dirs) = &ustream.receive().unwrap()[0] {
            assert_eq!(dirs.len(), 1); //Contains still one
        } else {
            panic!("Not a dirs");
        }
    }

    #[test]
    fn test_runner_read() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Send input before read, otherwise will block
        assert!(ustream.send(UserStreamMessage::Input(String::from("HI_THERE"))));
        //Read
        assert_eq!(runner.read(&mut core, Some(String::from("type something")), Some(5), Some(String::from("OUTPUT"))), 0);
        //Prompt is shown
        if let ShellStreamMessage::Output((stdout, stderr)) = &ustream.receive().unwrap()[0] {
            //Must be prompt
            assert_eq!(*stdout.as_ref().unwrap(), String::from("type something"));
        } else {
            panic!("Not an output");
        }
        //Get output
        assert_eq!(core.value_get(&String::from("OUTPUT")).unwrap(), String::from("HI_TH")); //Max size is 5, do you remember?
        //Let's try without option now
        assert!(ustream.send(UserStreamMessage::Input(String::from("HI_THERE"))));
        runner.read(&mut core, None, None, None);
        //Prompt is shown
        if let ShellStreamMessage::Output((stdout, stderr)) = &ustream.receive().unwrap()[0] {
            //Must be prompt
            assert_eq!(*stdout.as_ref().unwrap(), String::from(""));
        } else {
            panic!("Not an output");
        }
        assert_eq!(core.value_get(&String::from("REPLY")).unwrap(), String::from("HI_THERE")); //This time will be stored in reply
        //Let's try terminate, kill and other stuff
        core.value_unset(&String::from("REPLY"));
        assert!(ustream.send(UserStreamMessage::Kill));
        assert_eq!(runner.read(&mut core, None, None, None), 1);
        //Nothing to display
        assert!(core.value_get(&String::from("REPLY")).is_none());
        assert!(ustream.send(UserStreamMessage::Interrupt));
        assert_eq!(runner.read(&mut core, None, None, None), 1);
        //Nothing to display
        assert!(core.value_get(&String::from("REPLY")).is_none());
        assert!(ustream.send(UserStreamMessage::Signal(UnixSignal::Sigint)));
        assert_eq!(runner.read(&mut core, None, None, None), 1);
        //Nothing to display
        assert!(core.value_get(&String::from("REPLY")).is_none());
    }

    //TODO: set
    //TODO: source
    //TODO: while
    //TODO: test run

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
