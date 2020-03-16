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
    /// If wrkdir is None, home will be the working directory
    pub fn new(wrkdir: Option<PathBuf>, history_size: usize, parser: Box<dyn ParseStatement>) -> (ShellCore, UserStream) {
        let hostname: String = whoami::host();
        let username: String = whoami::username();
        let home: PathBuf = match home_dir() {
            Some(path) => PathBuf::from(path),
            None => PathBuf::from("/"),
        };
        //set Working directory here
        let wrkdir: PathBuf = match wrkdir {
            Some(dir) => dir,
            None => home.clone()
        };
        let _ = env::set_current_dir(wrkdir.as_path());
        //Get streams
        let (sstream, ustream) = streams::new_streams();
        //Instantiate and return new core
        let mut core = ShellCore {
            state: ShellState::Idle,
            exit_code: 0,
            execution_time: Duration::from_millis(0),
            pid: None,
            user: username,
            hostname: hostname,
            wrk_dir: wrkdir,
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
        //Push home to dirs
        core.pushd(core.home_dir.clone());
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
        let current_dir: PathBuf = self.wrk_dir.clone();
        match env::set_current_dir(directory.as_path()) {
            Ok(()) => {
                self.prev_dir = current_dir;
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
        self.dirs.push_front(dir);
    }

    //@! Exit
    
    /// ### exit
    /// 
    /// Terminate shell and exit
    pub(crate) fn exit(&mut self) {
        self.state = ShellState::Terminated;
        self.exit_code = 0;
        self.execution_time = Duration::from_secs(0);
        self.pid = None;
        self.user.clear();
        self.wrk_dir = PathBuf::from("");
        self.home_dir = PathBuf::from("");
        self.prev_dir = PathBuf::from("");
        self.hostname.clear();
        self.storage.clear();
        self.alias.clear();
        self.functions.clear();
        self.dirs.clear();
        self.history.clear();
        self.buf_in.clear();
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

    /// ### get_wrkdir
    /// 
    /// Returns the working directory.
    pub fn get_wrkdir(&self) -> PathBuf {
        self.wrk_dir.clone()
    }

    /// ### get_wrkdir_pretty
    /// 
    /// Returns the current working directory with resolved paths for prompt
    pub fn get_wrkdir_pretty(&self) -> String {
        let mut wrkdir: String = String::from(self.wrk_dir.clone().as_path().to_str().unwrap());
        let home_dir_str: String = String::from(self.home_dir.clone().as_path().to_str().unwrap());
        if wrkdir.starts_with(home_dir_str.as_str()) {
            wrkdir = wrkdir.replace(home_dir_str.as_str(), "~");
        }
        wrkdir
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

    /// ### history_get
    /// 
    /// Returns the entire history copy
    pub fn history_get(&self) -> VecDeque<String> {
        self.history.clone()
    }

    /// ### history_load
    /// 
    /// Load history
    /// NOTE: the maximum history size will still be the size provided at constructor
    pub fn history_load(&mut self, history: VecDeque<String>) {
        //Clear current history
        self.history.clear();
        //Iterate over provided history NOTE: VecDeque allocates capacity / 2 - 1
        let history_size: usize = (self.history.capacity() + 1) / 2;
        for (index, entry) in history.iter().enumerate() {
            if index >= history_size {
                break;
            }
            self.history.push_front(entry.clone());
        }
    }

    /// ### history_push
    /// 
    /// Push a new entry to the history.
    /// The entry is stored at the front of the history. The first the newest
    fn history_push(&mut self, expression: String) {
        //Check if history overflows the size
        let history_size: usize = (self.history.capacity() + 1) / 2;
        if self.history.len() + 1 > history_size {
            self.history.pop_back();
        }
        self.history.push_front(expression);
    }

    //@! Misc

    /// ### resolve_path
    /// 
    /// Resolve path
    pub(crate) fn resolve_path(&self, path: String) -> PathBuf {
        let mut resolved_path: String = path.clone();
        //If path starts with '~', replace ~ with home dir
        if resolved_path.starts_with("~") {
            let orig_path: String = resolved_path.clone();
            let path_after_tilde = &orig_path.as_str()[1..];
            resolved_path = String::from(self.home_dir.as_path().to_str().unwrap());
            resolved_path.push_str(path_after_tilde);
        }
        PathBuf::from(resolved_path)
    }

    //TODO: reverse search

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
        //If state is Waiting, concatenate bufin and stdin
        let stdin: String = match self.state {
            ShellState::Waiting => {
                self.buf_in.clone() + &stdin
            },
            _ => stdin
        };
        //Try to parse line
        match self.parser.parse(&stdin) {
            Ok(expression) => {
                //Push stdin to history
                self.history_push(stdin.clone());
                //Set state to Running
                self.state = ShellState::Running;
                //Instantiate runner
                let mut runner: ShellRunner = ShellRunner::new();
                //Set execution time
                self.execution_started = Instant::now();
                //Run expression
                let rc: u8 = runner.run(self, expression);
                self.execution_time = self.execution_started.elapsed();
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
                    _ => {
                        //Push stdin to history
                        self.history_push(stdin.clone());
                    }
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

}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::parsers::bash::Bash;
    use crate::ShellStatement;

    use std::process::Command;

    #[test]
    fn test_core_new() {
        //Instantiate Core
        let (core, _): (ShellCore, UserStream) = ShellCore::new(Some(PathBuf::from("/tmp/")), 2048, Box::new(Bash {}));
        //Verify core parameters
        assert_eq!(core.state, ShellState::Idle);
        assert_eq!(core.exit_code, 0);
        assert_eq!(core.execution_time.as_millis(), 0);
        assert!(core.pid.is_none());
        assert_eq!(core.hostname, whoami::host());
        assert_eq!(core.user, whoami::username());
        assert_eq!(core.wrk_dir, PathBuf::from("/tmp/"));
        assert_eq!(core.home_dir, home_dir().unwrap());
        assert_eq!(core.prev_dir, home_dir().unwrap());
        assert_eq!(core.storage.len(), 0);
        assert_eq!(core.alias.len(), 0);
        assert_eq!(core.functions.len(), 0);
        assert_eq!(core.dirs.len(), 1); //Contains home
        assert_eq!(core.dirs[0], home_dir().unwrap());
        assert_eq!(core.history.len(), 0);
        assert_eq!(core.buf_in.len(), 0);
        //Test without working directory
        let (core, _): (ShellCore, UserStream) = ShellCore::new(None, 2048, Box::new(Bash {}));
        assert_eq!(core.wrk_dir, core.home_dir);
    }

    #[test]
    fn test_core_alias() {
        //Instantiate Core
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 2048, Box::new(Bash {}));
        //Set alias
        core.set_alias(String::from("ll"), String::from("ls -l"));
        //Get alias
        assert_eq!(core.get_alias(&String::from("ll")).unwrap(), String::from("ls -l"));
        //Try to get unexisting alias
        assert!(core.get_alias(&String::from("foobar")).is_none());
        //Override alias
        core.set_alias(String::from("ll"), String::from("ls -l --color=auto"));
        assert_eq!(core.get_alias(&String::from("ll")).unwrap(), String::from("ls -l --color=auto"));
        //Add another alias and test get all alias
        core.set_alias(String::from("please"), String::from("sudo"));
        let alias_table: HashMap<String, String> = core.get_all_alias();
        //Verify table
        assert_eq!(alias_table.len(), 2);
        //Test unalias
        assert!(core.unalias(&String::from("ll")).is_some());
        assert!(core.unalias(&String::from("please")).is_some());
        assert!(core.unalias(&String::from("foobar")).is_none());
        assert!(core.get_alias(&String::from("ll")).is_none());
        assert!(core.get_alias(&String::from("please")).is_none());
    }

    #[test]
    fn test_core_change_dir() {
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(Some(PathBuf::from("/tmp/")), 2048, Box::new(Bash {}));
        //Change directory
        assert!(core.change_directory(PathBuf::from("/var/")).is_ok());
        //Verify current directory/previous directory
        assert_eq!(core.get_wrkdir(), PathBuf::from("/var/"));
        assert_eq!(core.get_prev_dir(), PathBuf::from("/tmp/"));
        //Change directory to unexisting directory
        assert_eq!(core.change_directory(PathBuf::from("/pippoland/")).err().unwrap(), ShellError::NoSuchFileOrDirectory);
        //Verify directories didn't change
        assert_eq!(core.get_wrkdir(), PathBuf::from("/var/"));
        assert_eq!(core.get_prev_dir(), PathBuf::from("/tmp/"));
        //Try to change directory to file
        let tmpfile: tempfile::NamedTempFile = create_tmpfile();
        assert_eq!(core.change_directory(PathBuf::from(tmpfile.path())).err().unwrap(), ShellError::NotADirectory);
        //Try to change directory to a directory where you can't enter
        let tmpdir: tempfile::TempDir = create_tmpdir();
        //Use chmod instead of set_mode because it just doesn't work...
        assert!(Command::new("chmod").args(&["000", tmpdir.path().to_str().unwrap()]).status().is_ok());
        //Okay, now try to change directory inside that directory
        assert_eq!(core.change_directory(PathBuf::from(tmpdir.path())).err().unwrap(), ShellError::PermissionDenied);
    }

    #[test]
    fn test_core_dirs() {
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 2048, Box::new(Bash {}));
        //Dirs contains only home once the object is istantiated
        assert_eq!(core.dirs().len(), 1);
        assert_eq!(core.dirs()[0], core.home_dir.clone());
        //Push directory
        core.pushd(PathBuf::from("/tmp/"));
        //Verify dirs; /tmp/ should be the first element
        assert_eq!(core.dirs().len(), 2);
        assert_eq!(core.dirs()[0], PathBuf::from("/tmp/"));
        assert_eq!(core.dirs()[1], core.home_dir.clone());
        //Popd front
        assert_eq!(core.popd_front().unwrap(), PathBuf::from("/tmp/"));
        assert_eq!(core.dirs().len(), 1);
        //Push directory
        core.pushd(PathBuf::from("/etc/"));
        //Popd back
        assert_eq!(core.popd_back().unwrap(), core.home_dir.clone());
        assert_eq!(core.dirs().len(), 1);
        assert_eq!(core.popd_back().unwrap(), PathBuf::from("/etc/"));
        //Try to pop when stack is empty
        assert!(core.popd_front().is_none());
    }

    #[test]
    fn test_core_get_files() {
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 2048, Box::new(Bash {}));
        //CD to tmp
        assert!(core.change_directory(PathBuf::from("/tmp/")).is_ok());
        //Create tmp files
        let tmpfile1: tempfile::NamedTempFile = create_tmpfile();
        let tmpfile2: tempfile::NamedTempFile = create_tmpfile();
        //List files
        let files: Vec<DirEntry> = core.get_files().unwrap();
        let mut matches: usize = 0;
        for file in files.iter() {
            if file.file_name() == tmpfile1.path().file_name().unwrap() {
                matches += 1;
            } else if file.file_name() == tmpfile2.path().file_name().unwrap() {
                matches += 1;
            }
        }
        assert_eq!(matches, 2);
    }

    #[test]
    fn test_core_exit() {
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 2048, Box::new(Bash {}));
        core.exit();
        assert_eq!(core.state, ShellState::Terminated);
        assert_eq!(core.user.len(), 0);
        assert_eq!(core.hostname.len(), 0);
        assert_eq!(core.home_dir, PathBuf::from(""));
        assert_eq!(core.prev_dir, PathBuf::from(""));
        assert_eq!(core.wrk_dir, PathBuf::from(""));
        assert_eq!(core.dirs.len(), 0);
    }

    #[test]
    fn test_core_functions() {
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 2048, Box::new(Bash {}));
        //Get unexisting function
        assert!(core.get_function(&String::from("testfunc")).is_none());
        //Create function
        let test_function: ShellExpression = ShellExpression {
            statements: vec![ShellStatement::Return(0)]
        };
        //Set function
        core.set_function(String::from("testfunc"), test_function);
        //Verify function exists
        assert!(core.get_function(&String::from("testfunc")).is_some());
    }

    #[test]
    fn test_core_getters() {
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 2048, Box::new(Bash {}));
        assert_eq!(core.get_home(), core.home_dir.clone());
        assert_eq!(core.get_prev_dir(), core.home_dir.clone());
        assert_eq!(core.get_wrkdir(), core.wrk_dir.clone());
        assert_eq!(core.get_wrkdir_pretty(), String::from("~"));
    }

    #[test]
    fn test_core_history() {
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 2, Box::new(Bash {})); //@! History size => 2
        //Verify history is empty
        assert_eq!(core.history_get().len(), 0);
        //Try to get from history
        assert!(core.history_at(0).is_none());
        //Push to history
        core.history_push(String::from("cargo test"));
        assert_eq!(core.history_at(0).unwrap(), String::from("cargo test"));
        //Push another line
        core.history_push(String::from("git status"));
        //Git status is now index 0, while cargo test is index 1
        assert_eq!(core.history_at(0).unwrap(), String::from("git status"));
        assert_eq!(core.history_at(1).unwrap(), String::from("cargo test"));
        //Push another line
        core.history_push(String::from("git commit -a -m 'random commit'"));
        //Max history length has been set to 2, to there should be only two lines
        assert_eq!(core.history.len(), 2);
        assert_eq!(core.history_at(0).unwrap(), String::from("git commit -a -m 'random commit'"));
        assert_eq!(core.history_at(1).unwrap(), String::from("git status"));
        //Clear history
        core.history.clear();
        //Load history
        let mut history: VecDeque<String> = VecDeque::with_capacity(3);
        history.push_back(String::from("command 1"));
        history.push_back(String::from("command 2"));
        history.push_back(String::from("command 3"));
        core.history_load(history);
        //Length must be 2, since history size is 2
        assert_eq!(core.history.len(), 2);
        assert_eq!(core.history_at(0).unwrap(), String::from("command 2"));
        assert_eq!(core.history_at(1).unwrap(), String::from("command 1"));
    }
    
    //TODO: Misc
    //TODO: readline
    //TODO: storage

    fn create_tmpfile() -> tempfile::NamedTempFile {
        tempfile::NamedTempFile::new().unwrap()
    }

    fn create_tmpdir() -> tempfile::TempDir {
        tempfile::TempDir::new().unwrap()
    }
}
