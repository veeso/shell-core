//! # Core
//!
//! `core` provides the implementation of Shell-Core struct

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

extern crate dirs;
extern crate whoami;

use crate::{ParserErrorCode, ParseStatement, ShellCore, ShellError, ShellExpression, ShellState, ShellRunner, UserStream};
use crate::streams;

use std::collections::{HashMap, VecDeque};
use dirs::home_dir;
use std::env;
use std::io::ErrorKind;
use std::fs::{DirEntry, read_dir};
use std::path::PathBuf;
use std::time::{Duration, Instant};

//Data types

impl ShellCore {

    /// ### new
    /// 
    /// Instantiate a new ShellCore. It also returns the User stream to be used during execution
    pub fn new(wrkdir: PathBuf, history_size: usize, parser: Box<dyn ParseStatement>) -> (ShellCore, UserStream) {
        let hostname: String = whoami::host();
        let username: String = whoami::username();
        let home: PathBuf = match home_dir() {
            Some(path) => PathBuf::from(path),
            None => PathBuf::from("/"),
        };
        //set Working directory here
        let _ = env::set_current_dir(wrkdir.as_path());
        //Get streams
        let (sstream, ustream) = streams::new_streams();
        //Instantiate and return new core
        let core = ShellCore {
            state: ShellState::Idle,
            exit_code: 0,
            execution_time: Duration::from_millis(0),
            pid: None,
            wrk_dir: wrkdir,
            user: username,
            hostname: hostname,
            home_dir: home.clone(),
            prev_dir: home,
            execution_started: Instant::now(),
            storage: HashMap::new(),
            alias: HashMap::new(),
            functions: HashMap::new(),
            dirs: VecDeque::with_capacity(255),
            history: VecDeque::with_capacity(history_size),
            parser: parser,
            buf_in: String::new(),
            sstream: sstream
        };
        //Return core and ustream
        (core, ustream)
    }

    //@! Alias

    /// ### get_all_alias
    /// 
    /// Returns the aliases in the current shell environment
    pub(crate) fn get_all_alias(&self) -> HashMap<String, String> {
        self.alias.clone()
    }

    /// ### get_alias
    /// 
    /// Returns the alias associated command with the provided name
    pub(crate) fn get_alias(&self, alias: &String) -> Option<String> {
        match self.alias.get(alias) {
            None => None,
            Some(s) => Some(s.clone())
        }
    }

    /// ### set_alias
    /// 
    /// Set an alias in the current shell environment
    pub(crate) fn set_alias(&mut self, alias: String, command: String) {
        self.alias.insert(alias, command);
    }

    /// ### unalias
    /// 
    /// Remove an alias from the current shell environment
    pub(crate) fn unalias(&mut self, alias: &String) -> Option<String> {
        self.alias.remove(alias)
    }

    //@!CD

    /// ### change_directory
    /// 
    /// Change current directory, the previous directory is stored as previous directory
    pub(crate) fn change_directory(&mut self, directory: PathBuf) -> Result<(), ShellError> {
        let current_prev_dir: PathBuf = self.prev_dir.clone();
        match env::set_current_dir(directory.as_path()) {
            Ok(()) => {
                self.prev_dir = current_prev_dir;
                self.wrk_dir = directory;
                Ok(())
            },
            Err(err) => match err.kind() {
                ErrorKind::PermissionDenied => Err(ShellError::PermissionDenied),
                ErrorKind::Other => Err(ShellError::NotADirectory),
                ErrorKind::NotFound => Err(ShellError::NoSuchFileOrDirectory),
                _ => Err(ShellError::Other)
            }
        }
    }

    //@! Directories

    /// ### dirs
    /// 
    /// Returns the current dir stack
    pub(crate) fn dirs(&self) -> VecDeque<PathBuf> {
        self.dirs.clone()
    }

    /// ### popd_back
    /// 
    /// Pop directory from back
    pub(crate) fn popd_back(&mut self) -> Option<PathBuf> {
        self.dirs.pop_back()
    }

    /// ### popd_front
    /// 
    /// Pop directory from front
    pub(crate) fn popd_front(&mut self) -> Option<PathBuf> {
        self.dirs.pop_front()
    }

    /// ### pushd
    /// 
    /// Push directory to back of directory queue
    /// If the capacity (255) is higher than length +1, the front directory will be popped first
    pub(crate) fn pushd(&mut self, dir: PathBuf) {
        if self.dirs.capacity() == self.dirs.len() + 1 {
            self.popd_front();
        }
        self.dirs.push_back(dir);
    }

    //@! Files

    /// ### get_files
    /// 
    /// Returns all the files in the current directory
    pub fn get_files(&self) -> Result<Vec<DirEntry>, ShellError> {
        let mut files: Vec<DirEntry> = Vec::new();
        let entries = match read_dir(self.wrk_dir.as_path()) {
            Ok(e) => e,
            Err(err) => return match err.kind() {
                ErrorKind::PermissionDenied => Err(ShellError::PermissionDenied),
                ErrorKind::Other => Err(ShellError::NotADirectory),
                ErrorKind::NotFound => Err(ShellError::NoSuchFileOrDirectory),
                _ => Err(ShellError::Other)
            }
        };
        for entry in entries {
            if let Some(file) = entry.ok() {
                files.push(file);
            }
        }
        Ok(files)
    }

    //@! Functions

    /// ### get_function
    /// 
    /// Returns a function from the current Shell Environment
    pub(crate) fn get_function(&self, name: &String) -> Option<ShellExpression> {
        match self.functions.get(name) {
            None => None,
            Some(f) => Some(f.clone())
        }
    }

    /// ### set_function
    /// 
    /// Set a new Shell Function
    pub(crate) fn set_function(&mut self, name: String, expression: ShellExpression) {
        self.functions.insert(name, expression);
    }

    //@! Exit
    
    /// ### exit
    /// 
    /// Terminate shell and exit
    pub(crate) fn exit(&mut self) {
        self.state = ShellState::Terminated;
        //TODO: clean all
        //TODO: reset memory
    }

    //@! Getters

    /// ### get_home
    /// 
    /// Returns the home directory
    pub fn get_home(&self) -> PathBuf {
        self.home_dir.clone()
    }

    /// ### get_prev_dir
    /// 
    /// Returns the previous directory
    pub fn get_prev_dir(&self) -> PathBuf {
        self.prev_dir.clone()
    }

    //@! History

    /// ### history_at
    /// 
    /// Get the command at a certain index of the history
    /// None is returned in case index is out of range
    pub fn history_at(&self, index: usize) -> Option<String> {
        match self.history.get(index) {
            Some(s) => Some(s.clone()),
            None => None
        }
    }

    /// ### history_load
    /// 
    /// Load history
    /// NOTE: the maximum history size will still be the size provided at constructor
    pub fn history_load(&mut self, history: VecDeque<String>) {
        //Clear current history
        self.history.clear();
        //Iterate over provided history
        let history_size: usize = self.history.capacity();
        for (index, entry) in history.iter().enumerate() {
            if index >= history_size {
                break;
            }
            self.history.push_back(entry.clone());
        }
    }

    //@! Readline

    /// ### readline
    /// 
    /// Read a line from input. 
    /// This function must be used to take care of parsing an expression and executing it if it's valid
    /// If the expression is not valid a ParserError is returned
    /// NOTE: if the expression is incomplete `ShellError::Parser::Incomplete` is returned BUT the current input is stored in the shell core.
    /// If you want to clear the buffer CALL flush()
    pub fn readline(&mut self, stdin: String) -> Result<u8, ShellError> {
        //Check shell is in Idle or Waiting
        if self.state != ShellState::Idle && self.state != ShellState::Waiting {
            return Err(ShellError::ShellNotInIdle)
        }
        //Try to parse line
        match self.parser.parse(&stdin) {
            Ok(expression) => {
                //Set state to Running
                self.state = ShellState::Running;
                //Run expression
                //Instantiate runner
                let mut runner: ShellRunner = ShellRunner::new();
                let rc: u8 = runner.run(self, expression);
                //Set state back to Idle
                self.state = ShellState::Idle;
                Ok(rc)
            },
            Err(err) => {
                match err.code {
                    ParserErrorCode::Incomplete => {
                        //Set state to Waiting and save stdin to buffer
                        self.state = ShellState::Waiting;
                        self.buf_in.push_str(stdin.as_str());
                    },
                    _ => {}
                }
                //Return error
                Err(ShellError::Parser(err))
            }
        }
    }

    /// ### flush
    /// 
    /// Flush the current buffer
    pub fn flush(&mut self) {
        self.buf_in.clear();
    }

    //@! Storage

    /// ### value_get
    /// 
    /// Get a value from the current shell environment, the value will be read from storage and if not found there in the environment storage
    pub(crate) fn value_get(&self, key: &String) -> Option<String> {
        if let Some(val) = self.storage_get(key) {
            Some(val)
        } else {
            //Try from environment
            self.environ_get(key)
        }
    }

    /// ### value_unset
    /// 
    /// Unset a value from storage and environ
    pub(crate) fn value_unset(&mut self, key: &String) {
        self.storage_unset(key);
        self.environ_unset(key);
    }

    /// ### environ_get
    /// 
    /// Get a variable from the environment
    fn environ_get(&self, key: &String) -> Option<String> {
        match env::var_os(key.as_str()) {
            None => None,
            Some(val) => {
                match val.into_string() {
                    Ok(s) => Some(s),
                    Err(_) => None
                }
            }
        }
    }

    /// ### environ_set
    /// 
    /// Set a value in the environment
    pub(crate) fn environ_set(&self, key: &String, value: &String) {
        env::set_var(key.clone(), value.clone());
    }

    /// ### environ_unset
    /// 
    /// Remove a variable from the environment
    fn environ_unset(&self, key: &String) {
        env::remove_var(key.clone());
    }

    /// ### storage_get
    /// 
    /// Get a value from the storage
    fn storage_get(&self, key: &String) -> Option<String> {
        match self.storage.get(key) {
            Some(v) => Some(v.clone()),
            None => None
        }
    }

    /// ### storage_set
    /// 
    /// Set a value in the Shell storage
    pub(crate) fn storage_set(&mut self, key: String, value: String) {
        self.storage.insert(key, value);
    }

    /// ### storage_unset
    /// 
    /// Unset a value from the storage
    fn storage_unset(&mut self, key: &String) {
        let _ = self.storage.remove(key);
    }

    //@! Time

    //TODO: get time

}

#[cfg(test)]
mod tests {

    //use super::*;

    //fn test_core_new() {
    //    let core: ShellCore = ShellCore::new(PathBuf::from("/tmp/"), 2048, );
    //}

    //TODO: alias (set, get, unalias, get all)
    //TODO: change_dir
    //TODO: get_files
    //TODO: get_home
    //TODO: get_previous_dir
    //TODO: history_load
    //TODO: history_get
    //TODO: value_get
    //TODO: value_unset
    //TODO: environ_set
}