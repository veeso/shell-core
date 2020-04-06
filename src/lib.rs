//! # Shell-Core
//!
//! `shell-core` is a library which provides the core functionality to implement a shell or to interface with one of them.

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

pub mod core;
mod runner;
pub mod streams;
pub mod parsers;
pub mod tasks;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use tasks::TaskManager;
use tasks::TaskError;
use tasks::Task;

/// ## ShellCore Struct
///
/// The shell core is the main shell core access point and contains all the data necessary to run a shell.
/// It also provides all the functions which a shell must provide
pub struct ShellCore {
    pub state: ShellState,                          //Shell state
    pub exit_code: u8,                              //Exitcode of the last executed command
    pub execution_time: Duration,                   //Execution time of the last executed command
    pub pid: Option<u32>,                           //Pid of the current process
    pub user: String,                               //Username
    pub hostname: String,                           //Hostname
    wrk_dir: PathBuf,                               //Working directory
    home_dir: PathBuf,                              //User home directory
    prev_dir: PathBuf,                              //Previous directory
    execution_started: Instant,                     //The instant when the last process was started
    storage: HashMap<String, String>,               //Session storage
    alias: HashMap<String, String>,                 //Aliases
    functions: HashMap<String, ShellExpression>,    //Functions
    dirs: VecDeque<PathBuf>,                        //Directory stack
    history: VecDeque<String>,                      //Shell history
    parser: Box<dyn ParseStatement>,                //Parser
    buf_in: String,                                 //Input buffer
    pub(crate) sstream: ShellStream                 //ShellStream
}

/// ## ShellState
///
/// The shell state describes the current shell state and is very useful to choose the behaviour of your shell (for example to print or not the prompt etc)
/// The states are described here below
///
/// Idle: the shell is doing nothing and is waiting for new commands
/// Waiting: the shell is waiting for further inputs (for example there is an incomplete expression in the buffer)
/// Running: the shell is running a process
/// Terminated: the shell has terminated
#[derive(Copy, Clone, PartialEq, std::fmt::Debug)]
pub enum ShellState {
    Idle,
    Waiting,
    Running,
    Terminated,
}

/// ## ShellError
/// 
/// The Shell Error represents an error raised by the Shell
#[derive(PartialEq, std::fmt::Debug)]
pub enum ShellError {
    NoSuchFileOrDirectory(PathBuf),
    NotADirectory(PathBuf),
    PermissionDenied(PathBuf),
    BadValue(String),           //Bad value error (e.g. bad variable name)
    OutOfHistoryRange,          //Out of History Range
    ShellNotInIdle,             //The shell must be in Idle state to perform this action
    DirsStackEmpty,             //Directory stack is empty
    NoSuchAlias(String),        //Alias doesn't exist
    TaskError(TaskError),       //Error reported by task; please refer to task error
    Parser(ParserError),        //Error reported by the Parser
    Math(MathError),            //Math error
    Other                       //Anything which is an undefined behaviour. This should never be raised
}

/// ## ShellFunction
/// 
/// The Shell Function represents a shell function, which is made up of a name and a vector of statements
#[derive(Clone, PartialEq, std::fmt::Debug)]
pub struct ShellExpression {
    statements: Vec<(ShellStatement, TaskRelation)>
}

/// ## ShellStatement
/// 
/// The shell statement represents a single statement for Shell
/// Tasks are pipelines
/// The Statements are:
/// - Alias: Association between name and command. Alias(None, None) => returns all aliases; Alias(Some, None) => returns alias command, Alias(Some, Some) => set alias
/// - Break: Break from current expression block if possible
/// - Case: case statement Case(Expression output to match, List of case => expression)
/// - Cd: change directory
/// - Continue: Continue in the current expression block if possible
/// - Exec: Perform Task
/// - ExecHistory: Perform command from history
/// - Exit: exit from expression
/// - Export: export a variable into environ
/// - For: For(String, Condition, Perform) iterator String: key name
/// - Function: defines a new function (Name, expression)
/// - If: If(Condition, Then, Else) condition
/// - Let: perform math operation to values Let(Result, operator1, operation, operator2)
/// - Popd: Pop directory from stack
/// - Pushd: Push directory to directory stack
/// - Read: Read command (Prompt, length, result_key)
/// - Return: return value
/// - Set: Set value into storage
/// - Source: source file
/// - Task: execute task
/// - Time: execute with time
/// - Unalias: remove an alias
/// - Value: simple value or key
/// - While: While(Condition, Perform) iterator
#[derive(Clone, std::fmt::Debug)]
pub enum ShellStatement {
    Alias(Option<String>, Option<String>),
    Break,
    Case(ShellExpression, Vec<(ShellExpression, ShellExpression)>),
    Cd(PathBuf),
    Continue,
    Dirs,
    Exec(Task),
    ExecHistory(usize),
    Exit(u8),
    Export(String, ShellExpression),
    For(String, ShellExpression, ShellExpression),
    Function(String, ShellExpression),
    If(ShellExpression, ShellExpression, Option<ShellExpression>),
    Let(String, ShellExpression, MathOperator, ShellExpression),
    PopdBack,
    PopdFront,
    Pushd(PathBuf),
    Read(Option<String>, Option<usize>, Option<String>),
    Return(u8),
    Set(String, ShellExpression),
    Source(PathBuf),
    Time(Task),
    Unalias(String),
    Value(String),
    While(ShellExpression, ShellExpression)
}

/// ## ShellRunner
/// 
/// The shell runner is the struct which takes care of running Shell Expressions
pub struct ShellRunner {
    buffer: Option<String>, //Input buffer
    exit_flag: Option<u8>,  //When active, exit from expression execution
    break_loop: bool        //Indicates whether parent loop has to be stopped
}

//@! Streams

/// ## ShellStream
/// 
/// The shell stream contains the streams used by the Shell Runner to communicate with the UserStream

pub(crate) struct ShellStream {
    receiver: mpsc::Receiver<UserStreamMessage>,    //Receive User messages
    sender: mpsc::Sender<ShellStreamMessage>,       //Sends Shell messages
}

/// ## UserStream
/// 
/// The user stream contains the streams used by the "user" to communicate with the ShellStream
pub struct UserStream {
    receiver: mpsc::Receiver<ShellStreamMessage>,   //Receive Shell messages
    sender: mpsc::Sender<UserStreamMessage>,        //Sends User messages
}

/// ## ShellStreamMessage
/// 
/// The shell stream message contains the messages which can be sent by the ShellCore to the "user"
#[derive(std::fmt::Debug)]
pub enum ShellStreamMessage {
    Output((Option<String>, Option<String>)),   //Shell Output (stdout, stderr)
    Error(ShellError),                          //Shell Error
    Dirs(VecDeque<PathBuf>),                    //Dirs output
    Alias(HashMap<String, String>),             //List of alias
    Time(Duration)                              //Command duration
}

/// ## UserStreamMessage
/// 
/// The User stream message contains the messages which can be sent by the "user" to the ShellCore during the execution
#[derive(std::fmt::Debug)]
pub enum UserStreamMessage {
    Input(String),          //Stdin
    Kill,                   //Kill NOTE: the kill is forwarded to the task
    Signal(UnixSignal),     //Signal NOTE: the signal is forwarded to the task
    Interrupt               //Interrupt shell runner execution. This interrupts the process too
}

//@! Redirection

/// ## FileRedirectionType
///
/// FileRedirectionType enum describes the redirect type for files
#[derive(Clone, PartialEq, std::fmt::Debug)]
pub enum FileRedirectionType {
    Truncate,
    Append,
}

/// ## Redirect
///
/// Redirect enum describes the redirect type of a command
#[derive(PartialEq, std::fmt::Debug)]
pub enum Redirection {
    Stdout,
    Stderr,
    File(String, FileRedirectionType),
}

//@! Maths

/// ## MathOperator
/// 
/// Math operator is used by Let statement
#[derive(Clone, PartialEq, std::fmt::Debug)]
pub enum MathOperator {
    And,
    Divide,
    Equal,
    Module,
    Multiply,
    NotEqual,
    Or,
    Power,
    ShiftLeft,
    ShiftRight,
    Subtract,
    Sum,
    Xor
}

/// ## MathError
/// 
/// MathError represents an error while performing some math operation
#[derive(Clone, PartialEq, std::fmt::Debug)]
pub enum MathError {
    DividedByZero,
    NegativePower
}

//@! Parser

/// ## ParserError
///
/// the Parser error struct describes the error returned by the parser
#[derive(PartialEq, std::fmt::Debug)]
pub struct ParserError {
    pub code: ParserErrorCode,
    pub message: String,
}

/// ## ParserErrorCode
///
/// The parser error code describes in a generic way the error type
///
/// - Incomplete: the statement is incomplete, further input is required. This should bring the Core to Waiting state
/// - BadToken: a bad token was found in the statement
#[derive(Copy, Clone, PartialEq, std::fmt::Debug)]
pub enum ParserErrorCode {
    Incomplete,
    BadToken,
    TooManyArgs,
}

/// ## ParseStatement
///
/// ParseStatement is the trait which must be implemented by a shell parser engine (e.g. bash, fish, zsh...)
pub trait ParseStatement {
    /// ### parse
    ///
    /// The parse method MUST parse the statement and IF VALID perform an action provided by the ShellCore
    ///
    /// e.g. if the statement is a variable assignment, the method MUST call the shellcore set method.
    /// Obviously, in case of error the core method hasn't to be called
    fn parse(&self, core: &ShellCore, statement: &String) -> Result<ShellExpression, ParserError>;
}

//@! Signals

/// ## UnixSignal
///
/// The UnixSignal enums represents the UNIX signals
#[derive(Copy, Clone, PartialEq, std::fmt::Debug)]
pub enum UnixSignal {
    Sighup = 1,
    Sigint = 2,
    Sigquit = 3,
    Sigill = 4,
    Sigtrap = 5,
    Sigabrt = 6,
    Sigbus = 7,
    Sigfpe = 8,
    Sigkill = 9,
    Sigusr1 = 10,
    Sigsegv = 11,
    Sigusr2 = 12,
    Sigpipe = 13,
    Sigalrm = 14,
    Sigterm = 15,
    Sigstkflt = 16,
    Sigchld = 17,
    Sigcont = 18,
    Sigstop = 19,
    Sigtstp = 20,
    Sigttin = 21,
    Sigttou = 22,
    Sigurg = 23,
    Sigxcpu = 24,
    Sigxfsz = 25,
    Sigvtalrm = 26,
    Sigprof = 27,
    Sigwinch = 28,
    Sigio = 29,
    Sigpwr = 30,
    Sigsys = 31
}

//@! Task
/// ## TaskRelation
///
/// The task relation describes the behaviour the task manager should apply in the task execution for the next command
///
/// - And: the next command is executed only if the current one has finished with success; the task result is Ok, if all the commands are successful
/// - Or: the next command is executed only if the current one has failed; the task result is Ok if one of the two has returned with success
/// - Pipe: the commands are chained through a pipeline. This means they're executed at the same time and the output of the first is redirect to the output of the seconds one
/// - Unrelated: the commands are executed without any relation. The return code is the return code of the last command executed
#[derive(Copy, Clone, PartialEq, std::fmt::Debug)]
pub enum TaskRelation {
    And,
    Or,
    Pipe,
    Unrelated,
}

//@! Traits implementation

impl Clone for Redirection {
    fn clone(&self) -> Redirection {
        match self {
            Redirection::File(file, file_mode) => {
                Redirection::File(file.clone(), file_mode.clone())
            }
            Redirection::Stderr => Redirection::Stderr,
            Redirection::Stdout => Redirection::Stdout,
        }
    }
}

impl PartialEq for ShellStatement {
    fn eq(&self, other: &Self) -> bool {
        match self {
            ShellStatement::Alias(name, value) => {
                if let ShellStatement::Alias(name_cmp, value_cmp) = other {
                    (name == name_cmp) && (value == value_cmp)
                } else {
                    false
                }
            },
            ShellStatement::Break => {
                if let ShellStatement::Break = other {
                    true
                } else {
                    false
                }
            },
            ShellStatement::Case(expr, cases) => {
                if let ShellStatement::Case(expr_cmp, cases_cmp) = other {
                    expr == expr_cmp && cases == cases_cmp
                } else {
                    false
                }
            },
            ShellStatement::Cd(path) => {
                if let ShellStatement::Cd(path_cmp) = other {
                    path == path_cmp
                } else {
                    false
                }
            },
            ShellStatement::Continue => {
                if let ShellStatement::Continue = other {
                    true
                } else {
                    false
                }
            },
            ShellStatement::Dirs => {
                if let ShellStatement::Dirs = other {
                    true
                } else {
                    false
                }
            },
            ShellStatement::Exec(t) => {
                if let ShellStatement::Exec(t_cmp) = other {
                    t.command == t_cmp.command
                } else {
                    false
                }
            },
            ShellStatement::ExecHistory(i) => {
                if let ShellStatement::ExecHistory(i_cmp) = other {
                    i == i_cmp
                } else {
                    false
                }
            },
            ShellStatement::Exit(rc) => {
                if let ShellStatement::Exit(rc_cmp) = other {
                    rc == rc_cmp
                } else {
                    false
                }
            },
            ShellStatement::Export(var, expr) => {
                if let ShellStatement::Export(var_cmp, expr_cmp) = other {
                    var == var_cmp && expr == expr_cmp
                } else {
                    false
                }
            },
            ShellStatement::For(var, cond, perform) => {
                if let ShellStatement::For(var_cmp, cond_cmp, perform_cmp) = other {
                    var == var_cmp && cond == cond_cmp && perform == perform_cmp
                } else {
                    false
                }
            },
            ShellStatement::Function(func, expr) => {
                if let ShellStatement::Function(func_cmp, expr_cmp) = other {
                    func == func_cmp && expr == expr_cmp
                } else {
                    false
                }
            },
            ShellStatement::If(condition, if_perform, else_perform) => {
                if let ShellStatement::If(condition_cmp, if_perform_cmp, else_perform_cmp) = other {
                    condition == condition_cmp && if_perform == if_perform_cmp && else_perform == else_perform_cmp
                } else {
                    false
                }
            },
            ShellStatement::Let(dest, op_a, op, op_b) => {
                if let ShellStatement::Let(dest_cmp, op_a_cmp, op_cmp, op_b_cmp) = other {
                    dest == dest_cmp && op_a == op_a_cmp && op == op_cmp && op_b == op_b_cmp
                } else {
                    false
                }
            },
            ShellStatement::PopdBack => {
                if let ShellStatement::PopdBack = other {
                    true
                } else {
                     false
                 }
            },
            ShellStatement::PopdFront => {
                if let ShellStatement::PopdFront = other {
                    true
                } else {
                    false
                }
            },
            ShellStatement::Pushd(path) => {
                if let ShellStatement::Pushd(path_cmp) = other {
                    path == path_cmp
                } else {
                    false
                }
            },
            ShellStatement::Read(prompt, length, result) => {
                if let ShellStatement::Read(prompt_cmp, length_cmp, result_cmp) = other {
                    prompt == prompt_cmp && length == length_cmp && result == result_cmp
                } else {
                    false
                }
            },
            ShellStatement::Return(rc) => {
                if let ShellStatement::Return(rc_cmp) = other {
                    rc == rc_cmp
                } else {
                    false
                }
            },
            ShellStatement::Set(var, expr) => {
                if let ShellStatement::Set(var_cmp, expr_cmp) = other {
                    var == var_cmp && expr == expr_cmp
                } else {
                    false
                }
            },
            ShellStatement::Source(path) => {
                if let ShellStatement::Source(path_cmp) = other {
                    path == path_cmp
                } else {
                    false
                }
            },
            ShellStatement::Time(t) => {
                if let ShellStatement::Time(t_cmp) = other {
                    t.command == t_cmp.command
                } else {
                    false
                }
            },
            ShellStatement::Unalias(alias) => {
                if let ShellStatement::Unalias(alias_cmp) = other {
                    alias == alias_cmp
                } else {
                    false
                }
            },
            ShellStatement::Value(val) => {
                if let ShellStatement::Value(val_cmp) = other {
                    val == val_cmp
                } else {
                    false
                }
            },
            ShellStatement::While(cond, perform) => {
                if let ShellStatement::While(cond_cmp, perform_cmp) = other {
                    cond == cond_cmp && perform == perform_cmp
                } else {
                    false
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_lib_shell_statement_eq() {
        //Alias
        assert_eq!(ShellStatement::Alias(None, None), ShellStatement::Alias(None, None));
        assert_ne!(ShellStatement::Alias(Some(String::from("foo")), Some(String::from("bar"))), ShellStatement::Alias(None, None));
        assert_ne!(ShellStatement::Alias(Some(String::from("foo")), Some(String::from("bar"))), ShellStatement::Break);
        //Break
        assert_eq!(ShellStatement::Break, ShellStatement::Break);
        assert_ne!(ShellStatement::Break, ShellStatement::Alias(None, None));
        //Case
        let case_match: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("1")), TaskRelation::Unrelated)]
        };
        let case_match2: ShellExpression = ShellExpression {
            statements: vec![(ShellStatement::Value(String::from("5")), TaskRelation::Unrelated)]
        };
        assert_eq!(ShellStatement::Case(case_match.clone(), vec![]), ShellStatement::Case(case_match.clone(), vec![]));
        assert_ne!(ShellStatement::Case(case_match.clone(), vec![]), ShellStatement::Case(case_match2.clone(), vec![]));
        assert_ne!(ShellStatement::Case(case_match.clone(), vec![]), ShellStatement::Break);
        //Cd
        assert_eq!(ShellStatement::Cd(PathBuf::from("/tmp/")), ShellStatement::Cd(PathBuf::from("/tmp/")));
        assert_ne!(ShellStatement::Cd(PathBuf::from("/tmp/")), ShellStatement::Cd(PathBuf::from("/home/")));
        assert_ne!(ShellStatement::Cd(PathBuf::from("/tmp/")), ShellStatement::Break);
        //Continue
        assert_eq!(ShellStatement::Continue, ShellStatement::Continue);
        assert_ne!(ShellStatement::Continue, ShellStatement::Alias(None, None));
        //Dirs
        assert_eq!(ShellStatement::Dirs, ShellStatement::Dirs);
        assert_ne!(ShellStatement::Dirs, ShellStatement::Alias(None, None));
        //Exec
        let task: Task = Task::new(vec![String::from("ls")], Redirection::Stdout, Redirection::Stderr);
        assert_eq!(ShellStatement::Exec(task.clone()), ShellStatement::Exec(task.clone()));
        assert_ne!(ShellStatement::Exec(task.clone()), ShellStatement::Exec(Task::new(vec![String::from("echo")], Redirection::Stdout, Redirection::Stderr)));
        assert_ne!(ShellStatement::Exec(task.clone()), ShellStatement::Break);
        //Exec history
        assert_eq!(ShellStatement::ExecHistory(8), ShellStatement::ExecHistory(8));
        assert_ne!(ShellStatement::ExecHistory(8), ShellStatement::ExecHistory(128));
        assert_ne!(ShellStatement::ExecHistory(8), ShellStatement::Break);
        //Exit
        assert_eq!(ShellStatement::Exit(0), ShellStatement::Exit(0));
        assert_ne!(ShellStatement::Exit(0), ShellStatement::Exit(128));
        assert_ne!(ShellStatement::Exit(0), ShellStatement::Break);
        //Export
        assert_eq!(ShellStatement::Export(String::from("foo"), ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}), ShellStatement::Export(String::from("foo"), ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}));
        assert_ne!(ShellStatement::Export(String::from("foo"), ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}), ShellStatement::Export(String::from("bar"), ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}));
        assert_ne!(ShellStatement::Export(String::from("foo"), ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}), ShellStatement::Break);
        //For
        assert_eq!(ShellStatement::For(String::from("VAR"), ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}, ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}), ShellStatement::For(String::from("VAR"), ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}, ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}));
        assert_ne!(ShellStatement::For(String::from("VAR"), ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}, ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}), ShellStatement::For(String::from("VAR2"), ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}, ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}));
        assert_ne!(ShellStatement::For(String::from("VAR"), ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}, ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}), ShellStatement::Break);
        //Function
        assert_eq!(ShellStatement::Function(String::from("foo"), ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}), ShellStatement::Function(String::from("foo"), ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}));
        assert_ne!(ShellStatement::Function(String::from("foo"), ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}), ShellStatement::Function(String::from("bar"), ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}));
        assert_ne!(ShellStatement::Function(String::from("foo"), ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}), ShellStatement::Break);
        //If
        assert_eq!(ShellStatement::If(ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}, ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}, None), ShellStatement::If(ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}, ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}, None));
        assert_ne!(ShellStatement::If(ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}, ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}, None), ShellStatement::If(ShellExpression {statements: vec![(ShellStatement::Return(3), TaskRelation::Unrelated)]}, ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}, None));
        assert_ne!(ShellStatement::If(ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}, ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}, None), ShellStatement::Break);
        //Let
        assert_eq!(ShellStatement::Let(String::from("TMP"), ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}, MathOperator::Sum, ShellExpression {statements: vec![(ShellStatement::Return(2), TaskRelation::Unrelated)]}), ShellStatement::Let(String::from("TMP"), ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}, MathOperator::Sum, ShellExpression {statements: vec![(ShellStatement::Return(2), TaskRelation::Unrelated)]}));
        assert_ne!(ShellStatement::Let(String::from("TMP"), ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}, MathOperator::Sum, ShellExpression {statements: vec![(ShellStatement::Return(2), TaskRelation::Unrelated)]}), ShellStatement::Let(String::from("TMP"), ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}, MathOperator::Subtract, ShellExpression {statements: vec![(ShellStatement::Return(2), TaskRelation::Unrelated)]}));
        assert_ne!(ShellStatement::Let(String::from("TMP"), ShellExpression {statements: vec![(ShellStatement::Return(0), TaskRelation::Unrelated)]}, MathOperator::Sum, ShellExpression {statements: vec![(ShellStatement::Return(2), TaskRelation::Unrelated)]}), ShellStatement::Break);
        //Popdback
        assert_eq!(ShellStatement::PopdBack, ShellStatement::PopdBack);
        assert_ne!(ShellStatement::PopdBack, ShellStatement::Break);
        //Popdfront
        assert_eq!(ShellStatement::PopdFront, ShellStatement::PopdFront);
        assert_ne!(ShellStatement::PopdFront, ShellStatement::Break);
        //Pushd
        assert_eq!(ShellStatement::Pushd(PathBuf::from("/tmp/")), ShellStatement::Pushd(PathBuf::from("/tmp/")));
        assert_ne!(ShellStatement::Pushd(PathBuf::from("/tmp/")), ShellStatement::Pushd(PathBuf::from("/home/")));
        assert_ne!(ShellStatement::Pushd(PathBuf::from("/tmp/")), ShellStatement::Break);
        //Read
        assert_eq!(ShellStatement::Read(None, None, None), ShellStatement::Read(None, None, None));
        assert_ne!(ShellStatement::Read(None, None, None), ShellStatement::Read(None, Some(32), None));
        assert_ne!(ShellStatement::Read(None, None, None), ShellStatement::Break);
        //Return
        assert_eq!(ShellStatement::Return(0), ShellStatement::Return(0));
        assert_ne!(ShellStatement::Return(0), ShellStatement::Return(2));
        assert_ne!(ShellStatement::Return(0), ShellStatement::Break);
        //Set
        assert_eq!(ShellStatement::Set(String::from("foo"), ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}), ShellStatement::Set(String::from("foo"), ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}));
        assert_ne!(ShellStatement::Set(String::from("foo"), ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}), ShellStatement::Set(String::from("foo"), ShellExpression {statements: vec![(ShellStatement::Value(String::from("5")), TaskRelation::Unrelated)]}));
        assert_ne!(ShellStatement::Set(String::from("foo"), ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}), ShellStatement::Break);
        //Source
        assert_eq!(ShellStatement::Source(PathBuf::from("/tmp/set.sh")), ShellStatement::Source(PathBuf::from("/tmp/set.sh")));
        assert_ne!(ShellStatement::Source(PathBuf::from("/tmp/set.sh")), ShellStatement::Source(PathBuf::from("/tmp/get.sh")));
        assert_ne!(ShellStatement::Source(PathBuf::from("/tmp/set.sh")), ShellStatement::Break);
        //Time
        assert_eq!(ShellStatement::Time(Task::new(vec![String::from("echo")], Redirection::Stdout, Redirection::Stderr)), ShellStatement::Time(Task::new(vec![String::from("echo")], Redirection::Stdout, Redirection::Stderr)));
        assert_ne!(ShellStatement::Time(Task::new(vec![String::from("echo")], Redirection::Stdout, Redirection::Stderr)), ShellStatement::Time(Task::new(vec![String::from("ls")], Redirection::Stdout, Redirection::Stderr)));
        assert_ne!(ShellStatement::Time(Task::new(vec![String::from("echo")], Redirection::Stdout, Redirection::Stderr)), ShellStatement::Break);
        //Unalias
        assert_eq!(ShellStatement::Unalias(String::from("ll")), ShellStatement::Unalias(String::from("ll")));
        assert_ne!(ShellStatement::Unalias(String::from("ll")), ShellStatement::Unalias(String::from("filesize")));
        assert_ne!(ShellStatement::Unalias(String::from("ll")), ShellStatement::Break);
        //Value
        assert_eq!(ShellStatement::Value(String::from("5")), ShellStatement::Value(String::from("5")));
        assert_ne!(ShellStatement::Value(String::from("5")), ShellStatement::Value(String::from("15")));
        assert_ne!(ShellStatement::Value(String::from("5")), ShellStatement::Break);
        //While
        assert_eq!(ShellStatement::While(ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}, ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}), ShellStatement::While(ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}, ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}));
        assert_ne!(ShellStatement::While(ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}, ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}), ShellStatement::While(ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}, ShellExpression {statements: vec![(ShellStatement::Value(String::from("2")), TaskRelation::Unrelated)]}));
        assert_ne!(ShellStatement::While(ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}, ShellExpression {statements: vec![(ShellStatement::Value(String::from("0")), TaskRelation::Unrelated)]}), ShellStatement::Value(String::from("5")));
    }
}
