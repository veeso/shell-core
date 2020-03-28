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
#[derive(Clone, std::fmt::Debug)]
pub struct ShellExpression {
    statements: Vec<ShellStatement>
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
    fn parse(&self, statement: &String) -> Result<ShellExpression, ParserError>;
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
