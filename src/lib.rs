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
    pub wrk_dir: PathBuf,                           //Working directory
    pub user: String,                               //Username
    pub hostname: String,                           //Hostname
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
    sstream: ShellStream                            //ShellStream
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
#[derive(std::fmt::Debug)]
pub enum ShellError {
    NoSuchFileOrDirectory,
    NotADirectory,
    PermissionDenied,
    ShellNotInIdle,         //The shell must be in Idle state to perform this action
    TaskError(TaskError),   //Error reported by task; please refer to task error
    Parser(ParserError),    //Error reported by the Parser
    Other                   //Anything which is an undefined behaviour. This should never be raised
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
/// - Alias: Association between name and command
/// - Break: Break from current expression block if possible
/// - Cd: change directory
/// - Continue: Continue in the current expression block if possible
/// - Exec: Perform Task
/// - ExecHistory: Perform command from history
/// - Exit: exit from expression
/// - Export: export a variable into environ
/// - For: For(Condition, Perform) iterator
/// - If: If(Condition, Then, Else) condition
/// - Popd: Pop directory from stack
/// - Pushd: Push directory to directory stack
/// - Read: Read command (Prompt, length)
/// - Return: return value
/// - Set: Set value into storage
/// - Source: source file
/// - Task: execute task
/// - Time: execute with time
/// - While: While(Condition, Perform) iterator
#[derive(Clone, std::fmt::Debug)]
pub enum ShellStatement {
    Alias(String, String),
    Break,
    Cd(PathBuf),
    Continue,
    Dirs,
    Exec(Task),
    ExecHistory(usize),
    Exit(u8),
    Export(String, String),
    For(Task, Vec<ShellStatement>),
    If(Task, Vec<ShellStatement>, Option<Vec<ShellStatement>>),
    Set(String, String),
    PopdBack,
    PopdFront,
    Pushd(PathBuf),
    Read(Option<String>, usize),
    Return(u8),
    Source(PathBuf),
    Time(Task),
    While(Task, Vec<ShellStatement>)
}

/// ## ShellRunner
/// 
/// The shell runner is the struct which takes care of running Shell Expressions
pub struct ShellRunner {
    task_manager: Option<TaskManager>,  //Task Manager
    buffer: String                      //Input buffer
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
pub enum ShellStreamMessage {
    Output((Option<String>, Option<String>)),   //Shell Output (stdout, stderr)
    Error(ShellError)                           //Shell Error
}

/// ## UserStreamMessage
/// 
/// The User stream message contains the messages which can be sent by the "user" to the ShellCore during the execution
pub enum UserStreamMessage {
    Input(String),          //Stdin
    Kill,                   //Kill
    Signal(UnixSignal),     //Signal
    Terminate               //Terminate shell runner execution
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

//@! Parser

/// ## ParserError
///
/// the Parser error struct describes the error returned by the parser
#[derive(std::fmt::Debug)]
pub struct ParserError {
    pub code: ParseErrorCode,
    pub message: String,
}

/// ## ParserErrorCode
///
/// The parser error code describes in a generic way the error type
///
/// - Incomplete: the statement is incomplete, further input is required. This should bring the Core to Waiting state
/// - BadToken: a bad token was found in the statement
#[derive(Copy, Clone, PartialEq, std::fmt::Debug)]
pub enum ParseErrorCode {
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
    Sighup,
    Sigint,
    Sigquit,
    Sigill,
    Sigtrap,
    Sigabrt,
    Sigbus,
    Sigfpe,
    Sigkill,
    Sigusr1,
    Sigsegv,
    Sigusr2,
    Sigpipe,
    Sigalrm,
    Sigterm,
    Sigstkflt,
    Sigchld,
    Sigcont,
    Sigstop,
    Sigtstp,
    Sigttin,
    Sigttou,
    Sigurg,
    Sigxcpu,
    Sigxfsz,
    Sigvtalrm,
    Sigprof,
    Sigwinch,
    Sigio,
    Sigpwr,
    Sigsys
}
