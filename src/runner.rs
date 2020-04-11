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

use crate::{FileRedirectionType, HistoryOptions, MathError, MathOperator, Redirection};
use crate::{ShellCore, ShellError, ShellExpression, ShellRunner, ShellStatement};
use crate::{ShellStream, ShellStreamMessage, UserStreamMessage};
use crate::{TaskManager, Task, TaskRelation};
use crate::tasks::{TaskError, TaskErrorCode, TaskMessageRx, TaskMessageTx};

use glob::glob;
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::fs::OpenOptions;
use std::io::BufRead;
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
    pub args: Vec<String>,
    pub redirection: Redirection,
}

impl ShellRunner {

    /// ### new
    /// 
    /// Instantiate a new ShellRunner
    pub(crate) fn new() -> ShellRunner {
        ShellRunner {
            buffer: None,
            exit_flag: None,
            break_loop: false
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
    /// Case may return None if nothing has been matched
    fn case(&mut self, core: &mut ShellCore, what: ShellExpression, cases: Vec<(ShellExpression, ShellExpression)>) -> Option<u8> {
        let mut exitcode: Option<u8> = None;
        let (_, output): (u8, String) = self.run_expression(core, what);
        //Output in
        for case in cases.iter() {
            let (_, case_match): (u8, String) = self.run_expression(core, case.0.clone());
            //If case match is equal to output, execute case perform
            if case_match == output || case_match == "\\*" {
                let (rc, _): (u8, String) = self.run_expression(core, case.1.clone());
                exitcode = Some(rc);
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
    fn exec(&mut self, core: &mut ShellCore, task: Task) -> (u8, String) {
        //Execution flags
        let mut brutally_terminated: bool = false;
        let mut relation_satisfied: bool = true;
        //Create command chain from Task
        let mut chain: TaskChain = self.chain_task(core, task);
        let mut rc: u8 = 0;
        let mut output: String = String::new(); //Output is both returned here and sent to the user
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
                    loop {
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
                                            //Send only if next relation is not Pipe (and stdout or stderr is some)
                                            if (stdout.is_some() || stderr.is_some()) && chain.next_relation != TaskRelation::Pipe {
                                                let _ = core.sstream.send(ShellStreamMessage::Output((stdout.clone(), stderr.clone())));
                                                output.push_str(stdout.as_ref().unwrap().as_str());
                                            } else if chain.next_relation == TaskRelation::Pipe && stdout.is_some() {
                                                self.buffer = Some(stdout.as_ref().unwrap().clone());
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
                        if task_manager.is_running() { //If running, sleep
                            //Sleep for 50ms
                            sleep(Duration::from_millis(50));
                        } else {
                            //Join process and break
                            rc = task_manager.join().unwrap_or(255);
                            break;
                        }
                    } //@! End of task manager loop
                    if brutally_terminated {
                        //Set exit flag to true and break
                        self.exit_flag = Some(rc);
                        break;
                    }
                } else if let Some(func) = chain.function { //@! Functions
                    //Prepare backup of tmp values
                    let mut current_tmp_values: Vec<String> = Vec::with_capacity(func.args.len());
                    //Set function arguments to storage
                    for (index, arg) in func.args.iter().enumerate() {
                        //Before setting new value, try to get current values. 
                        //NOTE: If this function is nested into another execution, they will be set
                        //Otherwise they won't, since at the end of this function, the values are freed
                        if let Some(val) = core.value_get(&index.to_string()) {
                            current_tmp_values.push(val);
                        }
                        core.storage_arg_set(index.to_string(), arg.clone());
                    }
                    //@! Execute function
                    let (exitcode, out): (u8, String) = self.run_expression(core, func.expression);
                    //remove arguments from storage
                    for (index, _) in func.args.iter().enumerate() {
                        core.value_unset(&index.to_string());
                    }
                    //Restore previous value if possible
                    for (index, val) in current_tmp_values.iter().enumerate() {
                        core.storage_arg_set(index.to_string(), val.clone());
                    }
                    //Push output to output and set rc
                    output.push_str(out.as_str());
                    rc = exitcode;
                    //Redirect output
                    if chain.next_relation == TaskRelation::Pipe {
                        //Push output to buffer
                        self.buffer = Some(out);
                    } else {
                        //Redirect output
                        if let Err(err) = self.redirect_function_output(&core.sstream, func.redirection, out) {
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
                relation_satisfied = self.is_relation_satisfied(rc, chain.prev_relation);
            } else {
                //Otherwise break
                break;
            }
        } //@! End of loop
        //Remove last new line from output
        if output.ends_with("\n") {
            let _ = output.pop();
        }
        (rc, output)
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
    /// 
    /// Separate functions from tasks into individual blocks.
    /// This function is kinda compley, I don't know exactly what it does, but works. Don't touch it.
    fn chain_task(&self, core: &mut ShellCore, mut head: Task) -> TaskChain {
        let mut chain: Option<TaskChain> = None;
        let mut previous_was_function: bool = false;
        let mut last_relation: TaskRelation = TaskRelation::Unrelated;
        let mut last_chain_block: Option<Task> = None;
        let mut chain_block_length: usize = 0;
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
                if head.command.len() > 1 {
                    for arg in head.command[1..].iter() {
                        argv.push(String::from(arg));
                    }
                }
            } else {
                //argv is head command
                argv = head.command.clone();
            }
            command = argv[0].clone();
            //@! Evaluate values
            if argv.len() > 1 {
                for arg in argv[1..].iter_mut() {
                    //Resolve value
                    *arg = self.eval_value(core, arg.to_string());
                }
            }
            //Push argv to task
            head.command = argv.clone();
            //Check if first element is a function
            if let Some(func) = core.function_get(&command) {
                //If it's a function, chain previous task block
                if let Some(mut chain_block) = last_chain_block.take() {
                    //Truncate task at index and get last relation
                    last_relation = chain_block.truncate(chain_block_length - 1);
                    chain_block_length = 0;
                    //Chain task
                    match chain.as_mut() {
                        None => {
                            chain = Some(TaskChain::new(Some(chain_block.clone()), None, TaskRelation::Unrelated));
                        },
                        Some(chain_obj) => {
                            chain_obj.chain(Some(chain_block.clone()), None, last_relation);
                        }
                    }
                }
                //If it's a function chain a function
                previous_was_function = true;
                match chain.as_mut() {
                    None => {
                        chain = Some(TaskChain::new(None, Some(Function::new(func, argv, head.stdout_redirection.clone())), TaskRelation::Unrelated));
                    },
                    Some(chain_obj) => {
                        chain_obj.chain(None, Some(Function::new(func, argv, head.stdout_redirection.clone())), last_relation);
                    }
                };
                last_relation = head.relation.clone();
                //Truncate task and relation
                if let Some(task) = head.next.clone() {
                    //Override head
                    //last_relation = head.relation.clone();
                    head = *task;
                } else { //No other tasks to iterate through
                    //Break
                    break;
                }
            } else { //Not a function
                //Incremenet chain block length
                chain_block_length += 1;
                //If previous was function or last chain block is none
                if previous_was_function || last_chain_block.is_none() {
                    previous_was_function = false;
                    last_chain_block = Some(head.clone());
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
        } //@! End of loop
        //If chain block is Some, finish chain
        if let Some(chain_block) = last_chain_block.take() {
            //Chain task
            match chain.as_mut() {
                None => {
                    chain = Some(TaskChain::new(Some(chain_block), None, TaskRelation::Unrelated));
                },
                Some(chain_obj) => {
                    chain_obj.chain(Some(chain_block.clone()), None, last_relation);
                }
            }
        }
        //Return chain
        chain.unwrap()
    }

    /// ### exec_time
    /// 
    /// Executes a command with duration
    fn exec_time(&mut self, core: &mut ShellCore, task: Task) -> (u8, String) {
        let t_start: Instant = Instant::now();
        let (rc, stdout): (u8, String) = self.exec(core, task);
        //Report execution time
        let exec_time: Duration = t_start.elapsed();
        //Report execution time
        if ! core.sstream.send(ShellStreamMessage::Time(exec_time)) {
            //Set exit flag
            self.exit_flag = Some(255);
        }
        (rc, stdout)
    }

    /// ### exit
    /// 
    /// Terminates Expression execution and shell
    fn exit(&mut self, _core: &mut ShellCore, exit_code: u8) {
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
    fn foreach(&mut self, core: &mut ShellCore, key: String, condition: ShellExpression, expression: ShellExpression) -> Option<u8> {
        //Get result of condition
        let mut exitcode: Option<u8> = None;
        let (rc, output): (u8, String) = self.run_expression(core, condition);
        if rc != 0 {
            return Some(1);
        }
        //Iterate over output split by whitespace
        for i in output.split_whitespace() {
            //Export key to storage
            core.storage_set(key.clone(), i.to_string());
            //Execute expression
            let (rc, _): (u8, String) = self.run_expression(core, expression.clone());
            exitcode = Some(rc);
            if self.break_loop {
                self.break_loop = false;
                break;
            }
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

    /// ### history
    /// 
    /// Handle history shell statement
    fn history(&mut self, core: &mut ShellCore, opt: HistoryOptions) -> u8 {
        match opt {
            HistoryOptions::Clear => {
                core.history_clear();
                0
            },
            HistoryOptions::Del(index) => {
                core.history_del(index);
                0
            },
            HistoryOptions::Print => {
                let mut out: String = String::new();
                let history: VecDeque<String> = core.history_get();
                for (index, line) in history.iter().enumerate() {
                    out += format!("{} {}\n", index, line).as_str();
                }
                if ! core.sstream.send(ShellStreamMessage::Output((Some(out), None))) {
                    //Set exit flag
                    self.exit_flag = Some(255);
                    return 255
                }
                0
            },
            HistoryOptions::Read(file) => {
                let file = match File::open(file) {
                    Ok(f) => f,
                    Err(_) => return 1
                };
                let lines = std::io::BufReader::new(file).lines();
                for line in lines {
                    if let Ok(line) = line {
                        //Push line to history
                        core.history_push(line.clone());
                    } else {
                        return 1
                    }
                }
                0
            },
            HistoryOptions::Write(file, trunc) => {
                let mut out: String = String::new();
                let history: VecDeque<String> = core.history_get();
                for line in history.iter() {
                    out += line;
                    out += "\n";
                }
                self.write_file(file, out, trunc)
            }
        }
    }

    /// ### ifcond
    /// 
    /// Perform if statement
    /// None is returned if no statement is executed
    fn ifcond(&mut self, core: &mut ShellCore, condition: ShellExpression, if_perform: ShellExpression, else_perform: Option<ShellExpression>) -> Option<u8> {
        //Get result of condition
        let mut exitcode: Option<u8> = None;
        let (rc, _): (u8, String) = self.run_expression(core, condition);
        //If rc is 0 => execute if perform
        if rc == 0 {
            //Execute expression
            let (rc, _) = self.run_expression(core, if_perform);
            exitcode = Some(rc);
        } else if let Some(else_perform) = else_perform {
            //Perform else if set
            let (rc, _) = self.run_expression(core, else_perform);
            exitcode = Some(rc);
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
                    return 1
                }
                operator1.pow(operator2 as u32)
            },
            MathOperator::ShiftLeft => {
                operator1 << operator2
            },
            MathOperator::ShiftRight => {
                operator1 >> operator2
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
            dirs.push_back(dir.clone());
            if ! core.sstream.send(ShellStreamMessage::Dirs(dirs)) {
                //Set exit flag
                self.exit_flag = Some(255);
            }
            //Change directory
            self.change_directory(core, dir)
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
            dirs.push_back(dir.clone());
            if ! core.sstream.send(ShellStreamMessage::Dirs(dirs)) {
                //Set exit flag
                self.exit_flag = Some(255);
            }
            //Change directory
            self.change_directory(core, dir)
        } else {
            1
        }
    }

    /// ### pushd
    /// 
    /// Execute pushd statement.
    fn pushd(&mut self, core: &mut ShellCore, dir: PathBuf) -> u8 {
        //Cd to dir
        if self.change_directory(core, dir.clone()) != 0 {
            return 1
        }
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
    fn while_loop(&mut self, core: &mut ShellCore, condition: ShellExpression, expression: ShellExpression) -> Option<u8> {
        let mut exitcode: Option<u8> = None;
        loop {
            let (rc, _): (u8, String) = self.run_expression(core, condition.clone());
            if rc != 0 { //If rc is NOT 0, break
                break;
            }
            //Otherwise perform expression
            let (rc, _) = self.run_expression(core, expression.clone());
            exitcode = Some(rc);
            if self.break_loop {
                self.break_loop = false;
                break;
            }
        }
        exitcode
    }

    /// ### write_file
    /// 
    /// Write file with a certain content
    fn write_file(&self, file: String, content: String, trunc: bool) -> u8 {
        match OpenOptions::new().create(true).write(true).append(!trunc).truncate(trunc).open(file.as_str()) {
            Ok(mut f) => {
                if let Err(e) = write!(f, "{}", content) {
                    1
                } else {
                    0
                }
            }
            Err(e) => {
                1
            }
        }
    }

    /// ### get_expression_str_value
    /// 
    /// Return the string output and the result of an expression.
    /// This function is very important since must be used by all the other statements which uses an expression (e.g. set, export, case, if...)
    fn run_expression(&mut self, core: &mut ShellCore, expression: ShellExpression) -> (u8, String) {
        let mut rc: u8 = 0;
        let mut output: String = String::new();
        //Iterate over expression
        let mut relation_satisfied: bool = true;
        //NOTE: the expression is executed as long as it's possible
        for statement in expression.statements.iter() {
            //Execute statement only if relation is satisfied
            if relation_satisfied {
                //Match statement and execute it
                match &statement.0 {
                    ShellStatement::Alias(name, cmd) => {
                        rc = self.alias(core, name.clone(), cmd.clone());
                    },
                    ShellStatement::Break => {
                        self.break_loop = true;
                        break; //Stop iterating
                    },
                    ShellStatement::Case(what, cases) => {
                        if let Some(exitcode) = self.case(core, what.clone(), cases.clone()) {
                            rc = exitcode;
                        }
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
                        let (exitcode, stdout): (u8, String) = self.exec(core, task.clone());
                        rc = exitcode;
                        output.push_str(stdout.as_str());
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
                        if let Some(exitcode) = self.foreach(core, what.clone(), when.clone(), perform.clone()) {
                            rc = exitcode;
                        }
                    },
                    ShellStatement::Function(name, expression) => {
                        rc = self.function(core, name.clone(), expression.clone());
                    },
                    ShellStatement::History(opt) => {
                        rc = self.history(core, opt.clone());
                    },
                    ShellStatement::If(what, perform_if, perform_else) => {
                        if let Some(exitcode) = self.ifcond(core, what.clone(), perform_if.clone(), perform_else.clone()) {
                            rc = exitcode;
                        }
                    },
                    ShellStatement::Let(dest, operator1, operation, operator2) => {
                        rc = self.let_perform(core, dest.clone(), operator1.clone(), operation.clone(), operator2.clone());
                    },
                    ShellStatement::Output(stdout, stderr) => {
                        //Send output (if not both are none)
                        if stdout.is_some() || stderr.is_some() {
                            core.sstream.send(ShellStreamMessage::Output((stdout.clone(), stderr.clone())));
                        }
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
                    ShellStatement::Rc(code) => {
                        rc = *code;
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
                        let (exitcode, stdout): (u8, String) = self.exec_time(core, task.clone());
                        rc = exitcode;
                        output.push_str(stdout.as_str());
                    },
                    ShellStatement::Unalias(alias) => {
                        rc = self.unalias(core, alias.clone());
                    },
                    ShellStatement::Value(val) => {
                        output = self.eval_value(core, val.clone());
                    },
                    ShellStatement::While(until, perform) => {
                        if let Some(exitcode) = self.while_loop(core, until.clone(), perform.clone()) {
                            rc = exitcode;
                        }
                    },
                    ShellStatement::WriteFile(file, content, trunc) => {
                        rc = self.write_file(file.clone(), content.clone(), *trunc);
                    }
                }
            }
            //Verify if relation is satisfied
            relation_satisfied = self.is_relation_satisfied(rc, statement.1);
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

    //@! Utils

    /// ### redirect_function_output
    ///
    /// Handle output redirections in a single method
    fn redirect_function_output(&self, _sstream: &ShellStream, redirection: Redirection, output: String) -> Result<(), ShellError> {
        match redirection {
            Redirection::Stdout => {
                //Send output
                //sstream.send(ShellStreamMessage::Output((Some(output), None))); NOTE: already sent by Exec in function
            },
            Redirection::Stderr => {
                //sstream.send(ShellStreamMessage::Output((None, Some(output)))); NOTE: already sent by Exec in function
            }
            Redirection::File(file, file_mode) => {
                match OpenOptions::new().create(true).write(true).append(file_mode == FileRedirectionType::Append).truncate(file_mode == FileRedirectionType::Truncate).open(file.as_str()) {
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

    /// ### is_relation_satisfied
    /// 
    /// Checks whether a relation between two task is satisfied
    fn is_relation_satisfied(&self, rc: u8, relation: TaskRelation) -> bool {
        match relation {
            TaskRelation::And => {
                //Set next to chain; if exitcode is 0, relation is satisfied
                if rc == 0 {
                    true
                } else {
                    false
                }
            },
            TaskRelation::Or => {
                //If exitcode is successful relation is unsatisfied
                if rc == 0 {
                    false
                } else {
                    true
                }
            },
            TaskRelation::Pipe | TaskRelation::Unrelated => {
                //Relation is always satisfied
                true
            }
        }
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
    pub(self) fn new(expression: ShellExpression, args: Vec<String>, redirection: Redirection) -> Function {
        Function {
            expression: expression,
            args: args,
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

    use std::fs::File;
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
    }

    #[test]
    fn test_runner_case() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Let's build our case statement - In this case 2 will be matched
        let case_match: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("2")), TaskRelation::Unrelated)]
        };
        let case0: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]
        };
        let case0_action: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]
        };
        let case1: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]
        };
        let case1_action: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Return(1), TaskRelation::Unrelated)]
        };
        let case2: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("2")), TaskRelation::Unrelated)]
        };
        let case2_action: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Return(2), TaskRelation::Unrelated)]
        };
        let case_any: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("\\*")), TaskRelation::Unrelated)]
        };
        let case_any_action: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Return(255), TaskRelation::Unrelated)]
        };
        let cases: Vec<(ShellExpression, ShellExpression)> = vec![(case0, case0_action), (case1, case1_action), (case2, case2_action), (case_any, case_any_action)];
        //Perform case
        //We expect 2 as rc, since the case 2 returns 2
        assert_eq!(runner.case(&mut core, case_match, cases.clone()).unwrap(), 2);
        //Let's try any case match (using value 40)
        let case_match: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("40")), TaskRelation::Unrelated)]
        };
        //We expect 255 as rc, since should match any case
        assert_eq!(runner.case(&mut core, case_match, cases.clone()).unwrap(), 255);
        //Let's try an unmatched case
        let case0: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]
        };
        let case0_action: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]
        };
        let cases: Vec<(ShellExpression, ShellExpression)> = vec![(case0, case0_action)];
        let case_match: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("1")), TaskRelation::Unrelated)]
        };
        assert!(runner.case(&mut core, case_match, cases).is_none());
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

    #[test]
    fn test_runner_chain_task() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Simple task
        let command: Vec<String> = vec![String::from("echo"), String::from("foo")];
        let mut sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let command: Vec<String> = vec![String::from("echo"), String::from("bar")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Unrelated,
        );
        //Chain task
        let chain: TaskChain = runner.chain_task(&mut core, sample_task);
        assert!(chain.task.is_some());
        assert!(chain.function.is_none());
        assert_eq!(chain.next_relation, TaskRelation::Unrelated);
        assert_eq!(chain.prev_relation, TaskRelation::Unrelated);
        assert!(chain.next.is_none());
        //@! Chain (task[2] + function + task[2])
        //Add a function to runner
        let expression: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]
        };
        runner.function(&mut core, String::from("myfunc"), expression);
        let command: Vec<String> = vec![String::from("echo"), String::from("foo")];
        let mut sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let command: Vec<String> = vec![String::from("echo"), String::from("bar")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::And, //and between echo1 and echo 2
        );
        let command: Vec<String> = vec![String::from("myfunc"), String::from("bar")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::And, //and between echo2 and myfunc
        );
        let command: Vec<String> = vec![String::from("cat"), String::from("/tmp/test.txt")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Or, //Or between myfunc and cat
        );
        let command: Vec<String> = vec![String::from("head"), String::from("-n"), String::from("1")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::Pipe,
        );
        let chain: TaskChain = runner.chain_task(&mut core, sample_task);
        //Let's see if it's correct
        assert!(chain.task.is_some());
        assert!(chain.task.clone().unwrap().next.is_some());
        assert!(chain.function.is_none());
        assert_eq!(chain.next_relation, TaskRelation::And);
        assert_eq!(chain.prev_relation, TaskRelation::Unrelated);
        assert!(chain.next.is_some());
        //VERY IMPORTANT! chain.task.next.next MUST be None
        assert!(chain.task.unwrap().next.unwrap().next.is_none());
        //Next is a function
        let chain: TaskChain = *chain.next.unwrap();
        assert!(chain.task.is_none());
        assert!(chain.function.is_some());
        assert_eq!(chain.function.as_ref().unwrap().args[0], String::from("myfunc"));
        assert_eq!(chain.function.as_ref().unwrap().args[1], String::from("bar"));
        assert_eq!(chain.next_relation, TaskRelation::Or);
        assert_eq!(chain.prev_relation, TaskRelation::And);
        assert!(chain.next.is_some());
        //Next is a task
        let chain: TaskChain = *chain.next.unwrap();
        assert!(chain.task.is_some());
        assert_eq!(chain.task.as_ref().unwrap().command[0], String::from("cat"));
        assert!(chain.task.as_ref().unwrap().next.is_some());
        assert!(chain.function.is_none());
        assert_eq!(chain.next_relation, TaskRelation::Unrelated);
        assert_eq!(chain.prev_relation, TaskRelation::Or);
        assert!(chain.next.is_none());

        //Task + function
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Simple task
        let command: Vec<String> = vec![String::from("echo"), String::from("foo")];
        let mut sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        //Add a function to runner
        let expression: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]
        };
        runner.function(&mut core, String::from("myfunc"), expression);
        let command: Vec<String> = vec![String::from("myfunc"), String::from("bar")];
        sample_task.new_pipeline(
            command,
            Redirection::Stdout,
            Redirection::Stderr,
            TaskRelation::And, //and between echo2 and myfunc
        );
        let chain: TaskChain = runner.chain_task(&mut core, sample_task);
        //Let's see if it's correct
        assert!(chain.task.is_some());
        assert!(chain.task.unwrap().next.is_none());
        assert!(chain.function.is_none());
        assert_eq!(chain.next_relation, TaskRelation::And);
        assert_eq!(chain.prev_relation, TaskRelation::Unrelated);
        assert!(chain.next.is_some());
        //Next is a function
        let chain: TaskChain = *chain.next.unwrap();
        assert!(chain.task.is_none());
        assert!(chain.function.is_some());
        assert_eq!(chain.function.as_ref().unwrap().args[0], String::from("myfunc"));
        assert_eq!(chain.function.as_ref().unwrap().args[1], String::from("bar"));
        assert_eq!(chain.next_relation, TaskRelation::Unrelated);
        assert_eq!(chain.prev_relation, TaskRelation::And);
        assert!(chain.next.is_none());
    }

    #[test]
    fn test_runner_exec_task_simple() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Prepare task to exec
        let task: Task = Task::new(vec![String::from("echo"), String::from("HELLO"), String::from("WORLD")], Redirection::Stdout, Redirection::Stderr);
        //Exec task
        let (rc, out): (u8, String) = runner.exec(&mut core, task);
        assert_eq!(rc, 0);
        assert_eq!(out, String::from("HELLO WORLD"));
        //One output
        let inbox: Vec<ShellStreamMessage> = ustream.receive().unwrap();
        println!("Exec Inbox: {:?}", inbox);
        assert_eq!(inbox.len(), 1);
        for (index, message) in inbox.iter().enumerate() {
            if let ShellStreamMessage::Output((stdout, _)) = message {
                match index {
                    _ => assert_eq!(*stdout.as_ref().unwrap(), String::from("HELLO WORLD\n")),
                }
            } else {
                panic!("Not an output message");
            }
        }
    }

    #[test]
    fn test_runner_exec_task_and_func() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Define a function which echoes the first provided argument
        let echo_task: Task = Task::new(vec![String::from("echo"), String::from("$1")], Redirection::Stdout, Redirection::Stderr);
        let myfunc: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Exec(echo_task), TaskRelation::Unrelated)]
        };
        //Save function
        assert_eq!(core.function_set(String::from("myecho"), myfunc), true);
        //Prepare task to exec
        let mut task: Task = Task::new(vec![String::from("echo"), String::from("HELLO"), String::from("WORLD")], Redirection::Stdout, Redirection::Stderr);
        //Chain myecho
        task.new_pipeline(vec![String::from("myecho"), String::from("HI")], Redirection::Stdout, Redirection::Stderr, TaskRelation::Unrelated);
        //Exec task
        let (rc, out): (u8, String) = runner.exec(&mut core, task);
        assert_eq!(rc, 0);
        assert_eq!(out, String::from("HELLO WORLD\nHI"));
        //Verify ustream messages
        let inbox: Vec<ShellStreamMessage> = ustream.receive().unwrap();
        //Two outputs
        println!("Exec Inbox: {:?}", inbox);
        assert_eq!(inbox.len(), 2);
        for (index, message) in inbox.iter().enumerate() {
            if let ShellStreamMessage::Output((stdout, _)) = message {
                match index {
                    0 => assert_eq!(*stdout.as_ref().unwrap(), String::from("HELLO WORLD\n")),
                    _ => assert_eq!(*stdout.as_ref().unwrap(), String::from("HI\n"))
                }
            } else {
                panic!("Not an output message");
            }
        }
    }

    #[test]
    fn test_runner_exec_pipeline_task_and_func() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Define a function which echoes the first provided argument
        let head_task: Task = Task::new(vec![String::from("head"), String::from("-n"), String::from("1")], Redirection::Stdout, Redirection::Stderr);
        let myfunc: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Exec(head_task), TaskRelation::Unrelated)]
        };
        //Save function
        assert_eq!(core.function_set(String::from("myhead"), myfunc), true);
        //Prepare task to exec
        let mut task: Task = Task::new(vec![String::from("echo"), String::from("HELLO")], Redirection::Stdout, Redirection::Stderr);
        //Chain myecho
        task.new_pipeline(vec![String::from("myhead")], Redirection::Stdout, Redirection::Stderr, TaskRelation::Pipe);
        //Exec task
        let (rc, out): (u8, String) = runner.exec(&mut core, task);
        assert_eq!(rc, 0);
        assert_eq!(out, String::from("HELLO"));
        //Verify ustream messages
        let inbox: Vec<ShellStreamMessage> = ustream.receive().unwrap();
        //Only final output
        println!("Exec Inbox: {:?}", inbox);
        assert_eq!(inbox.len(), 1);
        for (index, message) in inbox.iter().enumerate() {
            if let ShellStreamMessage::Output((stdout, _)) = message {
                match index {
                    _ => assert_eq!(*stdout.as_ref().unwrap(), String::from("HELLO\n"))
                }
            } else {
                panic!("Not an output message");
            }
        }
    }

    #[test]
    fn test_runner_exec_task_input() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Define a function which echoes the first provided argument
        let echo_task: Task = Task::new(vec![String::from("echo"), String::from("$1")], Redirection::Stdout, Redirection::Stderr);
        let myfunc: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Exec(echo_task), TaskRelation::Unrelated)]
        };
        //Save function
        assert_eq!(core.function_set(String::from("myecho"), myfunc), true);
        //Prepare task to exec
        let mut task: Task = Task::new(vec![String::from("head"), String::from("-n"), String::from("1")], Redirection::Stdout, Redirection::Stderr);
        //Chain myecho
        task.new_pipeline(vec![String::from("myecho"), String::from("HI")], Redirection::Stdout, Redirection::Stderr, TaskRelation::Unrelated);
        //Prepare Kill
        assert!(ustream.send(UserStreamMessage::Input(String::from("INPUT STRING\n"))));
        //Exec task
        let (rc, out): (u8, String) = runner.exec(&mut core, task);
        assert_eq!(rc, 0);
        assert_eq!(out, String::from("INPUT STRING\nHI"));
        //Verify ustream messages
        let inbox: Vec<ShellStreamMessage> = ustream.receive().unwrap();
        //One output
        println!("Exec Inbox: {:?}", inbox);
        assert_eq!(inbox.len(), 2);
        for (index, message) in inbox.iter().enumerate() {
            if let ShellStreamMessage::Output((stdout, _)) = message {
                match index {
                    0 => assert_eq!(*stdout.as_ref().unwrap(), String::from("INPUT STRING\n")),
                    _ => assert_eq!(*stdout.as_ref().unwrap(), String::from("HI\n"))
                }
            } else {
                panic!("Not an output message");
            }
        }
    }

    #[test]
    fn test_runner_exec_task_kill() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Define a function which echoes the first provided argument
        let echo_task: Task = Task::new(vec![String::from("echo"), String::from("$1")], Redirection::Stdout, Redirection::Stderr);
        let myfunc: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Exec(echo_task), TaskRelation::Unrelated)]
        };
        //Save function
        assert_eq!(core.function_set(String::from("myecho"), myfunc), true);
        //Prepare task to exec
        let mut task: Task = Task::new(vec![String::from("cat")], Redirection::Stdout, Redirection::Stderr);
        //Chain myecho
        task.new_pipeline(vec![String::from("myecho"), String::from("HI")], Redirection::Stdout, Redirection::Stderr, TaskRelation::Unrelated);
        //Prepare Kill
        assert!(ustream.send(UserStreamMessage::Kill));
        //Exec task (cat will be killed)
        let (rc, out): (u8, String) = runner.exec(&mut core, task);
        assert_eq!(rc, 0);
        assert_eq!(out, String::from("HI"));
        //Verify ustream messages
        let inbox: Vec<ShellStreamMessage> = ustream.receive().unwrap();
        //One output
        println!("Exec Inbox: {:?}", inbox);
        assert_eq!(inbox.len(), 1);
        for (index, message) in inbox.iter().enumerate() {
            if let ShellStreamMessage::Output((stdout, _)) = message {
                match index {
                    _ => assert_eq!(*stdout.as_ref().unwrap(), String::from("HI\n"))
                }
            } else {
                panic!("Not an output message");
            }
        }
    }

    #[test]
    fn test_runner_exec_task_signal() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Define a function which echoes the first provided argument
        let echo_task: Task = Task::new(vec![String::from("echo"), String::from("$1")], Redirection::Stdout, Redirection::Stderr);
        let myfunc: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Exec(echo_task), TaskRelation::Unrelated)]
        };
        //Save function
        assert_eq!(core.function_set(String::from("myecho"), myfunc), true);
        //Prepare task to exec
        let mut task: Task = Task::new(vec![String::from("cat")], Redirection::Stdout, Redirection::Stderr);
        //Chain myecho
        task.new_pipeline(vec![String::from("myecho"), String::from("HI")], Redirection::Stdout, Redirection::Stderr, TaskRelation::Unrelated);
        //Prepare Signal
        assert!(ustream.send(UserStreamMessage::Signal(UnixSignal::Sigint)));
        //Exec task (cat will be terminated)
        let (rc, out): (u8, String) = runner.exec(&mut core, task);
        assert_eq!(rc, 0);
        assert_eq!(out, String::from("HI"));
        //Verify ustream messages
        let inbox: Vec<ShellStreamMessage> = ustream.receive().unwrap();
        //One output
        println!("Exec Inbox: {:?}", inbox);
        assert_eq!(inbox.len(), 1);
        for (index, message) in inbox.iter().enumerate() {
            if let ShellStreamMessage::Output((stdout, _)) = message {
                match index {
                    _ => assert_eq!(*stdout.as_ref().unwrap(), String::from("HI\n"))
                }
            } else {
                panic!("Not an output message");
            }
        }
    }

    #[test]
    fn test_runner_exec_task_terminate() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Define a function which echoes the first provided argument
        let echo_task: Task = Task::new(vec![String::from("echo"), String::from("$1")], Redirection::Stdout, Redirection::Stderr);
        let myfunc: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Exec(echo_task), TaskRelation::Unrelated)]
        };
        //Save function
        assert_eq!(core.function_set(String::from("myecho"), myfunc), true);
        //Prepare task to exec
        let mut task: Task = Task::new(vec![String::from("cat")], Redirection::Stdout, Redirection::Stderr);
        //Chain myecho
        task.new_pipeline(vec![String::from("myecho"), String::from("HI")], Redirection::Stdout, Redirection::Stderr, TaskRelation::And);
        //Prepare Interrupt
        assert!(ustream.send(UserStreamMessage::Interrupt));
        //Exec task (cat will be terminated)
        let (rc, out): (u8, String) = runner.exec(&mut core, task);
        assert_eq!(rc, 130);
        assert_eq!(out, String::from(""));
        //Verify ustream messages
        let inbox: Vec<ShellStreamMessage> = ustream.receive().unwrap();
        //Zero output since task has been terminated
        println!("Exec Inbox: {:?}", inbox);
        assert_eq!(inbox.len(), 0);
    }

    #[test]
    fn test_runner_exec_relations_or_satisfied() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Define a function which echoes the first provided argument
        let echo_task: Task = Task::new(vec![String::from("echo"), String::from("$1")], Redirection::Stdout, Redirection::Stderr);
        let myfunc: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Exec(echo_task), TaskRelation::Unrelated)]
        };
        //Save function
        assert_eq!(core.function_set(String::from("myecho"), myfunc), true);
        //Prepare task to exec
        let mut task: Task = Task::new(vec![String::from("cat")], Redirection::Stdout, Redirection::Stderr);
        //Chain myecho
        task.new_pipeline(vec![String::from("myecho"), String::from("HI")], Redirection::Stdout, Redirection::Stderr, TaskRelation::Or);
        //Prepare Signal
        assert!(ustream.send(UserStreamMessage::Signal(UnixSignal::Sigint)));
        //Exec task (cat will be terminated)
        let (rc, out): (u8, String) = runner.exec(&mut core, task);
        assert_eq!(rc, 0);
        assert_eq!(out, String::from("HI"));
        //Verify ustream messages
        let inbox: Vec<ShellStreamMessage> = ustream.receive().unwrap();
        //One output
        println!("Exec Inbox: {:?}", inbox);
        assert_eq!(inbox.len(), 1);
        for (index, message) in inbox.iter().enumerate() {
            if let ShellStreamMessage::Output((stdout, _)) = message {
                match index {
                    _ => assert_eq!(*stdout.as_ref().unwrap(), String::from("HI\n"))
                }
            } else {
                panic!("Not an output message");
            }
        }
    }

    #[test]
    fn test_runner_exec_relations_or_unsatisfied() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Define a function which echoes the first provided argument
        let echo_task: Task = Task::new(vec![String::from("echo"), String::from("$1")], Redirection::Stdout, Redirection::Stderr);
        let myfunc: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Exec(echo_task), TaskRelation::Unrelated)]
        };
        //Save function
        assert_eq!(core.function_set(String::from("myecho"), myfunc), true);
        //Prepare task to exec
        let mut task: Task = Task::new(vec![String::from("echo"), String::from("OUTPUT")], Redirection::Stdout, Redirection::Stderr);
        //Chain myecho
        task.new_pipeline(vec![String::from("myecho"), String::from("HI")], Redirection::Stdout, Redirection::Stderr, TaskRelation::Or);
        //Exec task
        let (rc, out): (u8, String) = runner.exec(&mut core, task);
        assert_eq!(rc, 0);
        assert_eq!(out, String::from("OUTPUT"));
        //Verify ustream messages
        let inbox: Vec<ShellStreamMessage> = ustream.receive().unwrap();
        //One output (second task won't be executed)
        println!("Exec Inbox: {:?}", inbox);
        assert_eq!(inbox.len(), 1);
        for (index, message) in inbox.iter().enumerate() {
            if let ShellStreamMessage::Output((stdout, _)) = message {
                match index {
                    _ => assert_eq!(*stdout.as_ref().unwrap(), String::from("OUTPUT\n"))
                }
            } else {
                panic!("Not an output message");
            }
        }
    }

    #[test]
    fn test_runner_exec_relations_and_satisfied() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Define a function which echoes the first provided argument
        let echo_task: Task = Task::new(vec![String::from("echo"), String::from("$1")], Redirection::Stdout, Redirection::Stderr);
        let myfunc: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Exec(echo_task), TaskRelation::Unrelated)]
        };
        //Save function
        assert_eq!(core.function_set(String::from("myecho"), myfunc), true);
        //Prepare task to exec
        let mut task: Task = Task::new(vec![String::from("echo"), String::from("FOOBAR")], Redirection::Stdout, Redirection::Stderr);
        //Chain myecho
        task.new_pipeline(vec![String::from("myecho"), String::from("HI")], Redirection::Stdout, Redirection::Stderr, TaskRelation::And);
        //Prepare Signal
        assert!(ustream.send(UserStreamMessage::Signal(UnixSignal::Sigint)));
        //Exec task (cat will be terminated)
        let (rc, out): (u8, String) = runner.exec(&mut core, task);
        assert_eq!(rc, 0);
        assert_eq!(out, String::from("FOOBAR\nHI"));
        //Verify ustream messages
        let inbox: Vec<ShellStreamMessage> = ustream.receive().unwrap();
        //One output
        println!("Exec Inbox: {:?}", inbox);
        assert_eq!(inbox.len(), 2);
        for (index, message) in inbox.iter().enumerate() {
            if let ShellStreamMessage::Output((stdout, _)) = message {
                match index {
                    0 => assert_eq!(*stdout.as_ref().unwrap(), String::from("FOOBAR\n")),
                    _ => assert_eq!(*stdout.as_ref().unwrap(), String::from("HI\n"))
                }
            } else {
                panic!("Not an output message");
            }
        }
    }

    #[test]
    fn test_runner_exec_relations_and_unsatisfied() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Define a function which echoes the first provided argument
        let echo_task: Task = Task::new(vec![String::from("echo"), String::from("$1")], Redirection::Stdout, Redirection::Stderr);
        let myfunc: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Exec(echo_task), TaskRelation::Unrelated)]
        };
        //Save function
        assert_eq!(core.function_set(String::from("myecho"), myfunc), true);
        //Prepare task to exec
        let mut task: Task = Task::new(vec![String::from("cat")], Redirection::Stdout, Redirection::Stderr);
        //Chain myecho
        task.new_pipeline(vec![String::from("myecho"), String::from("HI")], Redirection::Stdout, Redirection::Stderr, TaskRelation::And);
        //Prepare Signal
        assert!(ustream.send(UserStreamMessage::Kill));
        //Exec task
        let (rc, out): (u8, String) = runner.exec(&mut core, task);
        assert_eq!(rc, 9);
        assert_eq!(out, String::from(""));
        //Verify ustream messages
        let inbox: Vec<ShellStreamMessage> = ustream.receive().unwrap();
        //Zero output, cat is terminated and second task doesn't start
        println!("Exec Inbox: {:?}", inbox);
        assert_eq!(inbox.len(), 0);
    }

    #[test]
    fn test_runner_history() {
        //TODO: requires readline
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Push entry to history
        core.history_push(String::from("echo foo"));
        core.history_push(String::from("echo bar"));
        //Try to clear
        assert_eq!(runner.history(&mut core, HistoryOptions::Clear), 0);
        assert_eq!(core.history.len(), 0);
        //Push entry to history
        core.history_push(String::from("echo foo"));
        core.history_push(String::from("echo bar"));
        //Delete
        assert_eq!(runner.history(&mut core, HistoryOptions::Del(0)), 0);
        assert_eq!(core.history.len(), 1);
        //Print
        assert_eq!(runner.history(&mut core, HistoryOptions::Print), 0);
        assert_eq!(ustream.receive().unwrap().len(), 1);
        //Write
        core.history_push(String::from("echo cac"));
        let tmpfile = create_tmpfile();
        let tmpfile_path: String = String::from(tmpfile.path().to_str().unwrap());
        assert_eq!(runner.history(&mut core, HistoryOptions::Write(tmpfile_path.clone(), true)), 0);
        core.history_clear();
        assert_eq!(core.history.len(), 0);
        //Reload history
        assert_eq!(runner.history(&mut core, HistoryOptions::Read(tmpfile_path)), 0);
        assert_eq!(core.history.len(), 2);
        /*
        assert_eq!(runner.exec_history(&mut core, 0), 0);
        //One output
        let inbox: Vec<ShellStreamMessage> = ustream.receive().unwrap();
        println!("Exec Inbox: {:?}", inbox);
        assert_eq!(inbox.len(), 1);
        for (index, message) in inbox.iter().enumerate() {
            if let ShellStreamMessage::Output((stdout, _)) = message {
                match index {
                    _ => assert_eq!(*stdout.as_ref().unwrap(), String::from("HELLO WORLD\n")),
                }
            } else {
                panic!("Not an output message");
            }
        }

        //Try to exec unexisting history entry
        assert_eq!(runner.exec_history(&mut core, 0), 128);
        //One output
        let inbox: Vec<ShellStreamMessage> = ustream.receive().unwrap();
        println!("Exec Inbox: {:?}", inbox);
        assert_eq!(inbox.len(), 1);
        for (index, message) in inbox.iter().enumerate() {
            if let ShellStreamMessage::Error(err) = message {
                match index {
                    _ => assert_eq!(*stdout.as_ref().unwrap(), String::from("HELLO WORLD\n")),
                }
            } else {
                panic!("Not an output message");
            }
        }
        */
    }

    #[test]
    fn test_runner_exec_time() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Prepare task to exec
        let task: Task = Task::new(vec![String::from("echo"), String::from("HELLO"), String::from("WORLD")], Redirection::Stdout, Redirection::Stderr);
        //Exec task
        let (rc, out): (u8, String) = runner.exec_time(&mut core, task);
        assert_eq!(rc, 0);
        assert_eq!(out, String::from("HELLO WORLD"));
        //One output
        let inbox: Vec<ShellStreamMessage> = ustream.receive().unwrap();
        println!("Exec Inbox: {:?}", inbox);
        assert_eq!(inbox.len(), 2);
        for (index, message) in inbox.iter().enumerate() {
            if index == 0 {
                if let ShellStreamMessage::Output((stdout, _)) = message {
                    assert_eq!(*stdout.as_ref().unwrap(), String::from("HELLO WORLD\n"));
                } else {
                    panic!("Not an output");
                }
            } else if index == 1 {
                if let ShellStreamMessage::Time(time) = message {
                    assert!(time.as_millis() < 1000 && time.as_millis() > 0);
                } else {
                    panic!("Not a time");
                }
            }
        }
    }

    #[test]
    fn test_runner_exit() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        runner.exit(&mut core, 0);
        assert_eq!(runner.exit_flag.unwrap(), 0);
    }

    #[test]
    fn test_runner_export() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, _ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Simple export
        assert_eq!(runner.export(&mut core, String::from("FOO"), ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("BAR")), TaskRelation::Unrelated)]
        }), 0);
        //Try bad variable name
        assert_eq!(runner.export(&mut core, String::from("5HIGH"), ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("BAR")), TaskRelation::Unrelated)]
        }), 1);
        //Verify value is exported
        assert_eq!(core.value_get(&String::from("FOO")).unwrap(), String::from("BAR"));
        //Complex export (with task)
        let command: Vec<String> = vec![String::from("echo"), String::from("5")];
        let sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let expression: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Exec(sample_task), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.export(&mut core, String::from("RESULT"), expression), 0);
        //Verify value is exported
        assert_eq!(core.value_get(&String::from("RESULT")).unwrap(), String::from("5"));
    }
    
    #[test]
    fn test_runner_foreach() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Let's create a temp dir with 4 files in it
        let (tmpdir, files): (tempfile::TempDir, Vec<String>) = create_tmp_dir_with_files(4);
        let file_case: String = format!("{}/*", tmpdir.path().display());
        //Prepare foreach
        let iterator: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(file_case.clone()), TaskRelation::Unrelated)]
        };
        let foreach_task: Task = Task::new(vec![String::from("echo"), String::from("$FILE")], Redirection::Stdout, Redirection::Stderr);
        let foreach_perform: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Exec(foreach_task), TaskRelation::Unrelated)]
        };
        //This for each will store each file contained in tmpdir into FILE; then for each entry echo $FILE will be performed
        assert_eq!(runner.foreach(&mut core, String::from("FILE"), iterator, foreach_perform).unwrap(), 0);
        //We should receive 4 messages in ustream
        let inbox: Vec<ShellStreamMessage> = ustream.receive().unwrap();
        assert_eq!(inbox.len(), 4);
        for (index, message) in inbox.iter().enumerate() {
            if let ShellStreamMessage::Output((stdout, _)) = message {
                let mut filename: String = stdout.as_ref().unwrap().clone();
                filename.pop(); //Remove newline
                //Verify filename is the same
                assert_eq!(filename, files[index].to_string());
            } else {
                panic!("Not an output message");
            }
        }
        //Test with break
        //Prepare foreach
        let iterator: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(file_case), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.foreach(&mut core, String::from("FILE"), iterator, ShellExpression {statements: vec![(ShellStatement::Break, TaskRelation::Unrelated)]}).unwrap(), 0);
        //Foreach in empty directory
        let tmpdir: tempfile::TempDir = create_tmp_dir();
        let file_case: String = format!("{}/*", tmpdir.path().display());
        //Prepare foreach
        let iterator: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(file_case.clone()), TaskRelation::Unrelated)]
        };
        let foreach_task: Task = Task::new(vec![String::from("echo"), String::from("$FILE")], Redirection::Stdout, Redirection::Stderr);
        let foreach_perform: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Exec(foreach_task), TaskRelation::Unrelated)]
        };
        //Must be None since there's no file in it
        assert!(runner.foreach(&mut core, String::from("FILE"), iterator, foreach_perform).is_none());
        //Foreach in not existing directory
        let iterator: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("/tmp/thisdirectorydoesnotexist/*")), TaskRelation::Unrelated)]
        };
        let foreach_task: Task = Task::new(vec![String::from("echo"), String::from("$FILE")], Redirection::Stdout, Redirection::Stderr);
        let foreach_perform: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Exec(foreach_task), TaskRelation::Unrelated)]
        };
        //Must be None, directory doesn't exist
        assert!(runner.foreach(&mut core, String::from("FILE"), iterator, foreach_perform).is_none());
    }

    #[test]
    fn test_runner_ifcond() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Let's try a simple if case without else
        let if_expression: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)] //This is OK, since returns 0
        };
        let if_perform: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Return(42), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.ifcond(&mut core, if_expression, if_perform, None).unwrap(), 42);
        //Let's try a simple if case without else, but if condition is false
        let if_expression: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Return(1), TaskRelation::Unrelated)] //This is Nok, since returns 1
        };
        let if_perform: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Return(42), TaskRelation::Unrelated)]
        };
        //Exitcode will be None
        assert!(runner.ifcond(&mut core, if_expression, if_perform, None).is_none());
        //Let's try a case with else, else is performed this time
        let if_expression: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Return(1), TaskRelation::Unrelated)] //This is Nok, since returns 1
        };
        let if_perform: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Return(42), TaskRelation::Unrelated)]
        };
        let else_perform: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Return(128), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.ifcond(&mut core, if_expression, if_perform, Some(else_perform)).unwrap(), 128);
    }

    #[test]
    fn test_runner_let() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Quick maths
        //And
        let operator1: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("32")), TaskRelation::Unrelated)]
        };
        let operator2: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("34")), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.let_perform(&mut core, String::from("RESULT"), operator1, MathOperator::And, operator2), 0);
        assert_eq!(core.value_get(&String::from("RESULT")).unwrap(), String::from("32"));
        //Divide
        let operator1: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("64")), TaskRelation::Unrelated)]
        };
        let operator2: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("32")), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.let_perform(&mut core, String::from("RESULT"), operator1, MathOperator::Divide, operator2), 0);
        assert_eq!(core.value_get(&String::from("RESULT")).unwrap(), String::from("2"));
        //Divide by 0
        let operator1: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("32")), TaskRelation::Unrelated)]
        };
        let operator2: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.let_perform(&mut core, String::from("RESULT"), operator1, MathOperator::Divide, operator2), 1);
        //Equal
        let operator1: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("16")), TaskRelation::Unrelated)]
        };
        let operator2: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("16")), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.let_perform(&mut core, String::from("RESULT"), operator1, MathOperator::Equal, operator2), 0);
        assert_eq!(core.value_get(&String::from("RESULT")).unwrap(), String::from("1"));
        //Equal (but not equal)
        let operator1: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("32")), TaskRelation::Unrelated)]
        };
        let operator2: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("34")), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.let_perform(&mut core, String::from("RESULT"), operator1, MathOperator::Equal, operator2), 0);
        assert_eq!(core.value_get(&String::from("RESULT")).unwrap(), String::from("0"));
        //Module
        let operator1: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("64")), TaskRelation::Unrelated)]
        };
        let operator2: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("24")), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.let_perform(&mut core, String::from("RESULT"), operator1, MathOperator::Module, operator2), 0);
        assert_eq!(core.value_get(&String::from("RESULT")).unwrap(), String::from("16"));
        //Multiply
        let operator1: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("4")), TaskRelation::Unrelated)]
        };
        let operator2: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("8")), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.let_perform(&mut core, String::from("RESULT"), operator1, MathOperator::Multiply, operator2), 0);
        assert_eq!(core.value_get(&String::from("RESULT")).unwrap(), String::from("32"));
        //NotEqual
        let operator1: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("2")), TaskRelation::Unrelated)]
        };
        let operator2: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("8")), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.let_perform(&mut core, String::from("RESULT"), operator1, MathOperator::NotEqual, operator2), 0);
        assert_eq!(core.value_get(&String::from("RESULT")).unwrap(), String::from("1"));
        //NotEqual (but equal)
        let operator1: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("32")), TaskRelation::Unrelated)]
        };
        let operator2: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("32")), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.let_perform(&mut core, String::from("RESULT"), operator1, MathOperator::NotEqual, operator2), 0);
        assert_eq!(core.value_get(&String::from("RESULT")).unwrap(), String::from("0"));
        //Or
        let operator1: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("16")), TaskRelation::Unrelated)]
        };
        let operator2: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("4")), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.let_perform(&mut core, String::from("RESULT"), operator1, MathOperator::Or, operator2), 0);
        assert_eq!(core.value_get(&String::from("RESULT")).unwrap(), String::from("20"));
        //Power
        let operator1: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("2")), TaskRelation::Unrelated)]
        };
        let operator2: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("3")), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.let_perform(&mut core, String::from("RESULT"), operator1, MathOperator::Power, operator2), 0);
        assert_eq!(core.value_get(&String::from("RESULT")).unwrap(), String::from("8"));
        //Power (negative power)
        let operator1: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("2")), TaskRelation::Unrelated)]
        };
        let operator2: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("-4")), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.let_perform(&mut core, String::from("RESULT"), operator1, MathOperator::Power, operator2), 1);
        //Shift Left
        let operator1: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("4")), TaskRelation::Unrelated)]
        };
        let operator2: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("8")), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.let_perform(&mut core, String::from("RESULT"), operator1, MathOperator::ShiftLeft, operator2), 0);
        assert_eq!(core.value_get(&String::from("RESULT")).unwrap(), String::from("1024"));
        //Right Shift
        let operator1: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("32")), TaskRelation::Unrelated)]
        };
        let operator2: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("34")), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.let_perform(&mut core, String::from("RESULT"), operator1, MathOperator::And, operator2), 0);
        assert_eq!(core.value_get(&String::from("RESULT")).unwrap(), String::from("32"));
        //Sum
        let operator1: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("5")), TaskRelation::Unrelated)]
        };
        let operator2: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("5")), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.let_perform(&mut core, String::from("RESULT"), operator1, MathOperator::Sum, operator2), 0);
        assert_eq!(core.value_get(&String::from("RESULT")).unwrap(), String::from("10"));
        //Xor
        let operator1: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("32")), TaskRelation::Unrelated)]
        };
        let operator2: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("4")), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.let_perform(&mut core, String::from("RESULT"), operator1, MathOperator::Xor, operator2), 0);
        assert_eq!(core.value_get(&String::from("RESULT")).unwrap(), String::from("36"));
    }

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
        //Verify working directory
        assert_eq!(core.get_wrkdir(), PathBuf::from("/tmp/"));
        //Try to pushd unexisting directory
        assert_eq!(runner.pushd(&mut core, PathBuf::from("/DOESNOTEXIST/")), 1);
        assert_eq!(core.get_wrkdir(), PathBuf::from("/tmp/"));
        let _ = ustream.receive();
        //Popd
        assert_eq!(runner.popd_back(&mut core), 0);
        if let ShellStreamMessage::Dirs(dirs) = &ustream.receive().unwrap()[0] {
            assert_eq!(dirs.len(), 1); //Contains home and tmp
            assert_eq!(*dirs[0], core.get_home());
        } else {
            panic!("Not a dirs");
        }
        //Verify working directory
        assert_eq!(core.get_wrkdir(), PathBuf::from(core.get_home()));
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
    fn test_runner_output() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));

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
        if let ShellStreamMessage::Output((stdout, _)) = &ustream.receive().unwrap()[0] {
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
        if let ShellStreamMessage::Output((stdout, _)) = &ustream.receive().unwrap()[0] {
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

    #[test]
    fn test_runner_set() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, _ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Simple export
        assert_eq!(runner.set(&mut core, String::from("FOO"), ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("BAR")), TaskRelation::Unrelated)]
        }), 0);
        assert_eq!(runner.set(&mut core, String::from("5HIGH"), ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("BAR")), TaskRelation::Unrelated)]
        }), 1);
        //Verify value is exported
        assert_eq!(core.value_get(&String::from("FOO")).unwrap(), String::from("BAR"));
        //Complex export (with task)
        let command: Vec<String> = vec![String::from("echo"), String::from("5")];
        let sample_task: Task = Task::new(command, Redirection::Stdout, Redirection::Stderr);
        let expression: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Exec(sample_task), TaskRelation::Unrelated)]
        };
        assert_eq!(runner.set(&mut core, String::from("RESULT"), expression), 0);
        //Verify value is exported
        assert_eq!(core.value_get(&String::from("RESULT")).unwrap(), String::from("5"));
    }

    //TODO: source (requires readline)

    #[test]
    fn test_runner_while() {
        let mut runner: ShellRunner = ShellRunner::new();
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Set value to 0
        core.storage_set(String::from("VALUE"), String::from("0"));
        //Prepare while (while [ $VALUE -ne 4 ]); do echo $VALUE; let VALUE=VALUE+1; done)
        let while_condition_task: Task = Task::new(vec![String::from("["), String::from("$VALUE"), String::from("-ne"), String::from("4"), String::from("]")], Redirection::Stdout, Redirection::Stderr);
        let while_condition: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Exec(while_condition_task), TaskRelation::Unrelated)] 
        };
        let echo_value_task: Task = Task::new(vec![String::from("echo"), String::from("$VALUE")], Redirection::Stdout, Redirection::Stderr);
        let operator1: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("$VALUE")), TaskRelation::Unrelated)]
        };
        let operator2: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("1")), TaskRelation::Unrelated)]
        };
        let while_perform: ShellExpression = ShellExpression { //Echo value; let value=value+1
            statements:vec![(ShellStatement::Exec(echo_value_task), TaskRelation::Unrelated), (ShellStatement::Let(String::from("VALUE"), operator1, MathOperator::Sum, operator2), TaskRelation::Unrelated)]
        };
        //Run while loop
        assert_eq!(runner.while_loop(&mut core, while_condition, while_perform).unwrap(), 0);
        //Verify we've received for outputs
        let inbox: Vec<ShellStreamMessage> = ustream.receive().unwrap();
        assert_eq!(inbox.len(), 4);
        for (index, message) in inbox.iter().enumerate() {
            if let ShellStreamMessage::Output((stdout, _)) = message {
                let mut value: String = stdout.as_ref().unwrap().clone();
                value.pop(); //Remove newline
                //Verify value is the same
                assert_eq!(value, index.to_string());
            } else {
                panic!("Not an output message");
            }
        }
        
        //Let's try a while with no cases
        let while_condition: ShellExpression = ShellExpression { //This will never run
            statements: vec![(ShellStatement::Return(1), TaskRelation::Unrelated)]
        };
        let while_perform: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Return(42), TaskRelation::Unrelated)]
        };
        //While result will be None
        assert!(runner.while_loop(&mut core, while_condition, while_perform).is_none());
        //Try while with Break (Mustn't block)
        assert_eq!(runner.while_loop(&mut core, ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}, ShellExpression {statements: vec![(ShellStatement::Break, TaskRelation::Unrelated)]}).unwrap(), 0);
    }

    #[test]
    fn test_runner_write_file() {
        let mut runner: ShellRunner = ShellRunner::new();
        let tmpfile = create_tmpfile();
        let tmpfile_path: String = String::from(tmpfile.path().to_str().unwrap());
        assert_eq!(runner.write_file(tmpfile_path, String::from("OUTPUT"), true), 0);
        assert_eq!(runner.write_file(String::from("/bin"), String::from("OUTPUT"), true), 1);
    }

    #[test]
    fn test_runner_run() {
        let mut runner: ShellRunner = ShellRunner::new();
        //Instantiate cores
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Prepare environment to run
        //Prepare case
        //Let's try an unmatched case
        let case0: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]
        };
        let case0_action: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]
        };
        let cases: Vec<(ShellExpression, ShellExpression)> = vec![(case0, case0_action)];
        let case_match: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("1")), TaskRelation::Unrelated)]
        };
        //Foreach
        //Let's create a temp dir with 2 files in it
        let (tmpdir, files): (tempfile::TempDir, Vec<String>) = create_tmp_dir_with_files(2);
        let file_case: String = format!("{}/*", tmpdir.path().display());
        //Prepare foreach
        let iterator: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(file_case), TaskRelation::Unrelated)]
        };
        let foreach_task: Task = Task::new(vec![String::from("echo"), String::from("$FILE")], Redirection::Stdout, Redirection::Stderr);
        let foreach_perform: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Exec(foreach_task), TaskRelation::Unrelated)]
        };
        //Push entry to history
        core.history_push(String::from("echo foobar"));
        //Send input (readline)
        ustream.send(UserStreamMessage::Input(String::from("INPUT\n")));
        //Prepare an expression with all the statements
        let expression: ShellExpression = ShellExpression {
            statements: vec![
                (ShellStatement::Read(Some(String::from(">>")), None, None), TaskRelation::Unrelated), //Read as first to not interfere with exec
                (ShellStatement::Alias(Some(String::from("ll")), Some(String::from("ls -l"))), TaskRelation::Unrelated),
                (ShellStatement::Case(case_match, cases), TaskRelation::Unrelated),
                (ShellStatement::Cd(PathBuf::from("/tmp/")), TaskRelation::Unrelated),
                (ShellStatement::Continue, TaskRelation::Unrelated),
                (ShellStatement::Dirs, TaskRelation::Unrelated),
                (ShellStatement::Exec(Task::new(vec![String::from("echo"), String::from("HELLO")], Redirection::Stdout, Redirection::Stderr)), TaskRelation::Unrelated),
                //ShellStatement::ExecHistory(0) TODO: requires readlin, TaskRelation::Unrelated)e
                (ShellStatement::Export(String::from("MYKEY"), ShellExpression {statements: vec![(ShellStatement::Value(String::from("MYVALUE")), TaskRelation::Unrelated)]}), TaskRelation::Unrelated),
                (ShellStatement::For(String::from("FILE"), iterator, foreach_perform), TaskRelation::Unrelated),
                (ShellStatement::Function(String::from("myecho"), ShellExpression { statements: vec![(ShellStatement::Exec(Task::new(vec![String::from("echo"), String::from("$1")], Redirection::Stdout, Redirection::Stderr)), TaskRelation::Unrelated)]}), TaskRelation::Unrelated),
                (ShellStatement::History(HistoryOptions::Clear), TaskRelation::Unrelated),
                (ShellStatement::If(ShellExpression {statements: vec![(ShellStatement::Value(String::from("1")), TaskRelation::Unrelated)]}, ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}, None), TaskRelation::Unrelated),
                (ShellStatement::Let(String::from("RESULT"), ShellExpression {statements: vec![(ShellStatement::Value(String::from("5")), TaskRelation::Unrelated)]}, MathOperator::Sum, ShellExpression {statements: vec![(ShellStatement::Value(String::from("2")), TaskRelation::Unrelated)]}), TaskRelation::Unrelated),
                (ShellStatement::Output(Some(String::from("STDOUT")), None), TaskRelation::Unrelated),
                (ShellStatement::Output(None, None), TaskRelation::Unrelated),
                (ShellStatement::Pushd(PathBuf::from("/tmp/")), TaskRelation::Unrelated),
                (ShellStatement::Pushd(PathBuf::from("/sbin/")), TaskRelation::Unrelated),
                (ShellStatement::PopdBack, TaskRelation::Unrelated),
                (ShellStatement::PopdFront, TaskRelation::Unrelated),
                (ShellStatement::Rc(55), TaskRelation::Unrelated),
                (ShellStatement::Set(String::from("YOURKEY"), ShellExpression {statements: vec![(ShellStatement::Value(String::from("YOURVALUE")), TaskRelation::Unrelated)]}), TaskRelation::Unrelated),
                //ShellStatement::Source(PathBuf::from("/tmp/stuff.sh")), TODO: requires readlin, TaskRelation::Unrelated
                (ShellStatement::Time(Task::new(vec![String::from("echo"), String::from("TIME")], Redirection::Stdout, Redirection::Stderr)), TaskRelation::Unrelated),
                (ShellStatement::Unalias(String::from("ll")), TaskRelation::Unrelated),
                (ShellStatement::While(ShellExpression {statements: vec![(ShellStatement::Value(String::from("1")), TaskRelation::Unrelated)]}, ShellExpression {statements: vec![(ShellStatement::Break, TaskRelation::Unrelated)]}), TaskRelation::Unrelated),
                (ShellStatement::WriteFile(String::from("/tmp/rust.out"), String::from("OUTPUT"), true), TaskRelation::Unrelated),
                (ShellStatement::Exit(1), TaskRelation::Unrelated)
            ]
        };
        //Run expression
        let rc: u8 = runner.run(&mut core, expression);
        assert_eq!(rc, 1); //Exit with rc 1
        /*
            Verify messages:

            - 0: Output (read prompt)
            - 1: Dirs
            - 2: Output: HELLO\n
            - 3: Output: file[0]
            - 4: Output: file[1]
            - 5: Output
            - 6: dirs
            - 7: dirs
            - 8: dirs
            - 9: dirs
            - 10: Output: TIME
            - 11: Time
        */
        let inbox: Vec<ShellStreamMessage> = ustream.receive().unwrap();
        println!("Runner::run INBOX {:?}", inbox);
        assert_eq!(inbox.len(), 12);
        for (index, message) in inbox.iter().enumerate() {
            match index {
                0 => {
                    if let ShellStreamMessage::Output((stdout, _))  = message {
                        assert_eq!(*stdout.as_ref().unwrap(), String::from(">>"));
                    } else {
                        panic!("Not an output");
                    }
                },
                1 => {
                    if let ShellStreamMessage::Dirs(d)  = message {
                        assert_eq!(d.len(), 1);
                    } else {
                        panic!("Not a dirs");
                    }
                },
                2 => {
                    if let ShellStreamMessage::Output((stdout, _))  = message {
                        assert_eq!(*stdout.as_ref().unwrap(), String::from("HELLO\n"));
                    } else {
                        panic!("Not an output");
                    }
                },
                3 => {
                    if let ShellStreamMessage::Output((stdout, _))  = message {
                        assert_eq!(*stdout.as_ref().unwrap(), format!("{}\n", files[0]));
                    } else {
                        panic!("Not an output");
                    }
                },
                4 => {
                    if let ShellStreamMessage::Output((stdout, _))  = message {
                        assert_eq!(*stdout.as_ref().unwrap(), format!("{}\n", files[1]));
                    } else {
                        panic!("Not an output");
                    }
                },
                5 => {
                    if let ShellStreamMessage::Output((stdout, stderr)) = message {
                        assert_eq!(*stdout.as_ref().unwrap(), String::from("STDOUT"));
                        assert!(stderr.is_none());
                    } else {
                        panic!("Not an output");
                    }
                },
                6 => {
                    if let ShellStreamMessage::Dirs(dirs) = message {
                        assert_eq!(dirs.len(), 2);
                        assert_eq!(dirs[0], PathBuf::from("/tmp/"));
                        assert_eq!(dirs[1], core.get_home());
                    } else {
                        panic!("Not a dirs");
                    }
                },
                7 => {
                    if let ShellStreamMessage::Dirs(dirs) = message {
                        assert_eq!(dirs.len(), 3);
                        assert_eq!(dirs[0], PathBuf::from("/sbin/"));
                        assert_eq!(dirs[1], PathBuf::from("/tmp/"));
                        assert_eq!(dirs[2], core.get_home());
                    } else {
                        panic!("Not a dirs");
                    }
                },
                8 => {
                    if let ShellStreamMessage::Dirs(dirs) = message {
                        assert_eq!(dirs.len(), 1);
                        assert_eq!(dirs[0], PathBuf::from(core.get_home()));
                    } else {
                        panic!("Not a dirs");
                    }
                },
                9 => {
                    if let ShellStreamMessage::Dirs(dirs) = message {
                        assert_eq!(dirs.len(), 1);
                        assert_eq!(dirs[0], PathBuf::from("/sbin/"));
                    } else {
                        panic!("Not a dirs");
                    }
                },
                10 => {
                    if let ShellStreamMessage::Output((stdout, _))  = message {
                        assert_eq!(*stdout.as_ref().unwrap(), String::from("TIME\n"));
                    } else {
                        panic!("Not an output");
                    }
                },
                11 => {
                    if let ShellStreamMessage::Time(t) = message {
                        assert!(t.as_millis() > 0 && t.as_millis() < 1000);
                    } else {
                        panic!("Not a time");
                    }
                },
                _ => panic!("Out of inbox range")
            }   
        }
        //Verify MYKEY
        assert_eq!(core.value_get(&String::from("MYKEY")).unwrap(), String::from("MYVALUE"));
        //Verify YOURKEY
        assert_eq!(core.value_get(&String::from("YOURKEY")).unwrap(), String::from("YOURVALUE"));
        //Verify RESULT
        assert_eq!(core.value_get(&String::from("RESULT")).unwrap(), String::from("7"));
        //Verify Alias
        assert!(core.alias_get(&String::from("ll")).is_none());
    }
    
    #[test]
    fn test_runner_run_with_relations() {
        let mut runner: ShellRunner = ShellRunner::new();
        //Instantiate cores
        let (mut core, ustream): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Prepare expression
        //cd /tmp/ (OK) AND cd /fjggtt/ (FAILS) AND cd /bin/ (won't run) OR echo FOO OR echo BAR
        //Expected result:
        // Wrkdir: /tmp/
        // Output: FOO
        let expression: ShellExpression = ShellExpression {
            statements: vec![
                (ShellStatement::Cd(PathBuf::from("/tmp/")), TaskRelation::And),
                (ShellStatement::Cd(PathBuf::from("/fjggtt/")), TaskRelation::And),
                (ShellStatement::Cd(PathBuf::from("/bin/")), TaskRelation::Or),
                (ShellStatement::Exec(Task::new(vec![String::from("echo"), String::from("FOO")], Redirection::Stdout, Redirection::Stderr)), TaskRelation::Or),
                (ShellStatement::Exec(Task::new(vec![String::from("echo"), String::from("BAR")], Redirection::Stdout, Redirection::Stderr)), TaskRelation::Unrelated),
                (ShellStatement::Cd(PathBuf::from("/onett/")), TaskRelation::Unrelated),
            ]
        };
        let rc: u8 = runner.run(&mut core, expression);
        let inbox: Vec<ShellStreamMessage> = ustream.receive().unwrap();
        println!("Runner::run_with_relations INBOX {:?}", inbox);
        assert_eq!(inbox.len(), 3);
        if let ShellStreamMessage::Error(err) = inbox.get(0).unwrap() {
            assert_eq!(discriminant(err), discriminant(&ShellError::NoSuchFileOrDirectory(PathBuf::from("/fjggtt/"))));
        } else {
            panic!("Not an error");
        }
        if let ShellStreamMessage::Output((stdout, _)) = inbox.get(1).unwrap() {
            assert_eq!(*stdout.as_ref().unwrap(), String::from("FOO\n"));
        } else {
            panic!("Not an output");
        }
        if let ShellStreamMessage::Error(err) = inbox.get(0).unwrap() {
            assert_eq!(discriminant(err), discriminant(&ShellError::NoSuchFileOrDirectory(PathBuf::from("/onett/"))));
        } else {
            panic!("Not an error");
        }
        assert_eq!(core.get_wrkdir(), PathBuf::from("/tmp/"));
        //Rc will be 1
        assert_eq!(rc, 1);
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
        let expression: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Exit(0), TaskRelation::Unrelated)]
        };
        //Instantiate function
        let argv: Vec<String> = vec![String::from("hi")];
        let function: Function = Function::new(expression, argv, Redirection::Stdout);
        assert_eq!(function.redirection, Redirection::Stdout);
        assert_eq!(function.expression.statements.len(), 1);
        assert_eq!(function.args.len(), 1);
        assert_eq!(discriminant(&function.expression.statements[0].0), discriminant(&ShellStatement::Exit(0)));
    }

    #[test]
    fn test_runner_chain() {
        //Instantiate an expression
        let expression: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Exit(0), TaskRelation::Unrelated)]
        };
        //Instantiate function
        let argv: Vec<String> = vec![String::from("hi")];
        let function: Function = Function::new(expression, argv, Redirection::Stdout);
        let mut chain: TaskChain = TaskChain::new(None, Some(function), TaskRelation::Unrelated);
        //Verify constructor
        assert_eq!(chain.prev_relation, TaskRelation::Unrelated);
        assert_eq!(chain.next_relation, TaskRelation::Unrelated);
        assert!(chain.next.is_none());
        assert!(chain.function.is_some());
        assert!(chain.task.is_none());
        //Prepare stuff to chain a new object
        let expression: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("BAR")), TaskRelation::Unrelated)]
        };
        let expression: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Set(String::from("FOO"), expression), TaskRelation::Unrelated)]
        };
        let argv: Vec<String> = vec![String::from("hi")];
        let function: Function = Function::new(expression, argv, Redirection::Stdout);
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
        let expression: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Read(None, None, None), TaskRelation::Unrelated)]
        };
        let argv: Vec<String> = vec![String::from("hi")];
        let function: Function = Function::new(expression, argv, Redirection::Stdout);
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

    //@! Utils
    fn create_tmp_dir_with_files(amount: usize) -> (tempfile::TempDir, Vec<String>) {
        let tmpdir: tempfile::TempDir = tempfile::TempDir::new().unwrap();
        let mut files: Vec<String> = Vec::with_capacity(amount);
        for i in 0..amount {
            let filename: String = format!("{}/file_{}.txt", tmpdir.path().display(), i.to_string());
            files.push(filename.clone());
            let mut file = File::create(filename).unwrap();
            let _ = file.write_all(b"Hello World!");
        }
        (tmpdir, files)
    }

    fn create_tmp_dir() -> tempfile::TempDir {
        tempfile::TempDir::new().unwrap()
    }

    fn create_tmpfile() -> tempfile::NamedTempFile {
        tempfile::NamedTempFile::new().unwrap()
    }

}
