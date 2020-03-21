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

    /// ### alias_get_all
    /// 
    /// Returns the aliases in the current shell environment
    pub(crate) fn alias_get_all(&self) -> HashMap<String, String> {
        self.alias.clone()
    }

    /// ### alias_get
    /// 
    /// Returns the alias associated command with the provided name
    pub(crate) fn alias_get(&self, alias: &String) -> Option<String> {
        match self.alias.get(alias) {
            None => None,
            Some(s) => Some(s.clone())
        }
    }

    /// ### alias_set
    /// 
    /// Set an alias in the current shell environment
    pub(crate) fn alias_set(&mut self, alias: String, command: String) {
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
                ErrorKind::PermissionDenied => Err(ShellError::PermissionDenied(directory.clone())),
                ErrorKind::Other => Err(ShellError::NotADirectory(directory.clone())),
                ErrorKind::NotFound => Err(ShellError::NoSuchFileOrDirectory(directory.clone())),
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
    pub fn exit(&mut self) {
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
        self.get_files_in(self.wrk_dir.clone())
    }

    /// ### get_files
    /// 
    /// Returns all the files in the provided directory
    pub fn get_files_in(&self, path: PathBuf) -> Result<Vec<DirEntry>, ShellError> {
        let mut files: Vec<DirEntry> = Vec::new();
        let entries = match read_dir(path.as_path()) {
            Ok(e) => e,
            Err(err) => return match err.kind() {
                ErrorKind::PermissionDenied => Err(ShellError::PermissionDenied(path.clone())),
                ErrorKind::Other => Err(ShellError::NotADirectory(path.clone())),
                ErrorKind::NotFound => Err(ShellError::NoSuchFileOrDirectory(path.clone())),
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

    /// ### function_get
    /// 
    /// Returns a function from the current Shell Environment
    pub(crate) fn function_get(&self, name: &String) -> Option<ShellExpression> {
        match self.functions.get(name) {
            None => None,
            Some(f) => Some(f.clone())
        }
    }

    /// ### function_set
    /// 
    /// Set a new Shell Function
    pub(crate) fn function_set(&mut self, name: String, expression: ShellExpression) {
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

    /// ### reverse_search
    /// 
    /// Search for a string match in the history
    /// This functions returns the list of all the first 'max_entries' in the history
    /// NOTE: this function may allocate a lot of memory, use reverse_search_hit to preserve memory
    pub fn reverse_search(&self, needle: &String, max_entries: Option<usize>) -> Option<Vec<String>> {
        let mut matches: Vec<String> = Vec::new();
        //Iterate over history
        for entry in self.history.iter() {
            if entry.as_str().contains(needle.as_str()) {
                matches.push(entry.clone());
            }
            //Check if matches length has reached max entries
            if let Some(max_entries) = max_entries {
                if matches.len() == max_entries {
                    break;
                }
            }
        }
        //Return matches or None
        match matches.len() {
            0 => None,
            _ => Some(matches)
        }
    }

    /// ### reverse_search_hits
    /// 
    /// Search for a string match in the history
    /// This functions returns the nth match in the history from the newer to the oldest entry in the history
    /// Matches start from 0 to n. If hit is higher than the maximum match, None is returned
    pub fn reverse_search_hits(&self, needle: &String, hit: usize) -> Option<String> {
        let mut matchnth: usize = 0;
        //Iterate over history
        for entry in self.history.iter() {
            if entry.as_str().contains(needle.as_str()) {
                //If matchnth == hit, return entry
                if matchnth == hit {
                    return Some(entry.clone())
                }
                //Increment matchnth
                matchnth += 1;
            }
        }
        None
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
                //Set rc to storage
                self.storage_set(String::from("status"), rc.to_string());
                self.storage_set(String::from("?"), rc.to_string());
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

    /// ### eval
    /// 
    /// Evaluate an expression
    pub fn eval(&mut self, expression: ShellExpression) -> u8 {
        //Instantiate runner
        let mut runner: ShellRunner = ShellRunner::new();
        //Eval
        let rc: u8 = runner.run(self, expression);
        //Set rc to storage
        self.storage_set(String::from("status"), rc.to_string());
        self.storage_set(String::from("?"), rc.to_string());
        rc
    }

    /// ### source
    /// 
    /// Source file
    pub fn source(&mut self, file: PathBuf) -> Result<u8, ShellError> {
        //Read file
        let file_content: String = match std::fs::read_to_string(file.as_path()) {
            Ok(cnt) => cnt,
            Err(err) => match err.kind() {
                ErrorKind::NotFound => return Err(ShellError::NoSuchFileOrDirectory(file.clone())),
                ErrorKind::PermissionDenied => return Err(ShellError::PermissionDenied(file.clone())),
                _ => return Err(ShellError::Other)
            }
        };
        //Parse file
        match self.parser.parse(&file_content) {
            Ok(expression) => {
                Ok(self.eval(expression))
            },
            Err(err) => Err(ShellError::Parser(err))
        }
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
    pub(crate) fn environ_set(&self, key: String, value: String) {
        env::set_var(key, value);
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
        let (core, _): (ShellCore, UserStream) = ShellCore::new(Some(PathBuf::from("/tmp/")), 128, Box::new(Bash {}));
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
        assert_eq!(core.exit_code, 0);
        assert_eq!(core.alias.len(), 0);
        assert_eq!(core.functions.len(), 0);
        assert_eq!(core.dirs.len(), 1); //Contains home
        assert_eq!(core.dirs[0], home_dir().unwrap());
        assert_eq!(core.history.len(), 0);
        assert_eq!(core.buf_in.len(), 0);
        //Test without working directory
        let (core, _): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        assert_eq!(core.wrk_dir, core.home_dir);
    }

    #[test]
    fn test_core_alias() {
        //Instantiate Core
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Set alias
        core.alias_set(String::from("ll"), String::from("ls -l"));
        //Get alias
        assert_eq!(core.alias_get(&String::from("ll")).unwrap(), String::from("ls -l"));
        //Try to get unexisting alias
        assert!(core.alias_get(&String::from("foobar")).is_none());
        //Override alias
        core.alias_set(String::from("ll"), String::from("ls -l --color=auto"));
        assert_eq!(core.alias_get(&String::from("ll")).unwrap(), String::from("ls -l --color=auto"));
        //Add another alias and test get all alias
        core.alias_set(String::from("please"), String::from("sudo"));
        let alias_table: HashMap<String, String> = core.alias_get_all();
        //Verify table
        assert_eq!(alias_table.len(), 2);
        //Test unalias
        assert!(core.unalias(&String::from("ll")).is_some());
        assert!(core.unalias(&String::from("please")).is_some());
        assert!(core.unalias(&String::from("foobar")).is_none());
        assert!(core.alias_get(&String::from("ll")).is_none());
        assert!(core.alias_get(&String::from("please")).is_none());
    }

    #[test]
    fn test_core_change_dir() {
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(Some(PathBuf::from("/tmp/")), 128, Box::new(Bash {}));
        //Change directory
        assert!(core.change_directory(PathBuf::from("/var/")).is_ok());
        //Verify current directory/previous directory
        assert_eq!(core.get_wrkdir(), PathBuf::from("/var/"));
        assert_eq!(core.get_prev_dir(), PathBuf::from("/tmp/"));
        //Change directory to unexisting directory
        assert_eq!(core.change_directory(PathBuf::from("/pippoland/")).err().unwrap(), ShellError::NoSuchFileOrDirectory(PathBuf::from("/pippoland/")));
        //Verify directories didn't change
        assert_eq!(core.get_wrkdir(), PathBuf::from("/var/"));
        assert_eq!(core.get_prev_dir(), PathBuf::from("/tmp/"));
        //Try to change directory to file
        let tmpfile: tempfile::NamedTempFile = create_tmpfile();
        assert_eq!(core.change_directory(PathBuf::from(tmpfile.path())).err().unwrap(), ShellError::NotADirectory(PathBuf::from(tmpfile.path())));
        //Try to change directory to a directory where you can't enter
        let tmpdir: tempfile::TempDir = create_tmpdir();
        //Use chmod instead of set_mode because it just doesn't work...
        assert!(Command::new("chmod").args(&["000", tmpdir.path().to_str().unwrap()]).status().is_ok());
        //Okay, now try to change directory inside that directory
        assert_eq!(core.change_directory(PathBuf::from(tmpdir.path())).err().unwrap(), ShellError::PermissionDenied(PathBuf::from(tmpdir.path())));
    }

    #[test]
    fn test_core_dirs() {
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
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
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
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
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
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
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Get unexisting function
        assert!(core.function_get(&String::from("testfunc")).is_none());
        //Create function
        let test_function: ShellExpression = ShellExpression {
            statements: vec![ShellStatement::Return(0)]
        };
        //Set function
        core.function_set(String::from("testfunc"), test_function);
        //Verify function exists
        assert!(core.function_get(&String::from("testfunc")).is_some());
    }

    #[test]
    fn test_core_getters() {
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
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
    
    #[test]
    fn test_core_misc_resolve_path() {
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 2048, Box::new(Bash {}));
        assert_eq!(core.resolve_path(String::from("/tmp/")), PathBuf::from("/tmp/"));
        assert_eq!(core.resolve_path(String::from("~")), core.home_dir.clone());
        let mut dev_home_path: PathBuf = core.home_dir.clone();
        dev_home_path.push("develop/Rust/");
        assert_eq!(core.resolve_path(String::from("~/develop/Rust/")), dev_home_path);
    }

    #[test]
    fn test_core_misc_reverse_search() {
        //Push some entries to the history
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 64, Box::new(Bash {}));
        core.history_push(String::from("git status"));
        core.history_push(String::from("git commit -a"));
        core.history_push(String::from("git commit README.md"));
        core.history_push(String::from("git push origin master"));
        core.history_push(String::from("ll"));
        core.history_push(String::from("cd /tmp/"));
        core.history_push(String::from("mc -e test.cpp"));
        core.history_push(String::from("g++ test.cpp -o test"));
        core.history_push(String::from("./test"));
        core.history_push(String::from("cd -"));
        //Try the reverse search with hits
        assert_eq!(core.reverse_search_hits(&String::from("git"), 0).unwrap(), String::from("git push origin master"));
        assert_eq!(core.reverse_search_hits(&String::from("git"), 1).unwrap(), String::from("git commit README.md"));
        assert_eq!(core.reverse_search_hits(&String::from("git"), 2).unwrap(), String::from("git commit -a"));
        assert_eq!(core.reverse_search_hits(&String::from("git"), 3).unwrap(), String::from("git status"));
        //Some other research with hits
        assert_eq!(core.reverse_search_hits(&String::from("commit"), 1).unwrap(), String::from("git commit -a"));
        //Try a research with hits out of range
        assert!(core.reverse_search_hits(&String::from("commit"), 3).is_none());
        //Try a research with hits of something unmatched
        assert!(core.reverse_search_hits(&String::from("foobar"), 0).is_none());
        //Try research with full stack
        let matches: Vec<String> = core.reverse_search(&String::from("git"), None).unwrap();
        assert_eq!(matches.len(), 4);
        assert_eq!(matches[0], String::from("git push origin master"));
        assert_eq!(matches[3], String::from("git status"));
        //Try research with full stack and max entries
        let matches: Vec<String> = core.reverse_search(&String::from("git"), Some(2)).unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0], String::from("git push origin master"));
        assert_eq!(matches[1], String::from("git commit README.md"));
        //Try research with full stack of something unmatched
        assert!(core.reverse_search(&String::from("foobar"), None).is_none());
    }

    //TODO: readline
    //TODO: eval
    //TODO: source

    #[test]
    fn test_core_storage() {
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 128, Box::new(Bash {}));
        //Try to get from storage a value which is not set
        assert!(core.value_get(&String::from("FOO")).is_none());
        assert!(core.storage_get(&String::from("FOO")).is_none());
        //Set a value in the storage
        core.storage_set(String::from("FOO"), String::from("BAR"));
        //Verify value is in the storage
        assert_eq!(core.value_get(&String::from("FOO")).unwrap(), String::from("BAR"));
        //Unset value
        core.value_unset(&String::from("FOO"));
        assert!(core.value_get(&String::from("FOO")).is_none());
        //Set value in the environ
        core.environ_set(String::from("MYKEY"), String::from("305"));
        assert_eq!(core.value_get(&String::from("MYKEY")).unwrap(), String::from("305"));
        //Set a value in the storage with name MYKEY
        core.storage_set(String::from("MYKEY"), String::from("SATURN"));
        //Now MYKEY, if retrieved should be SATURN
        assert_eq!(core.value_get(&String::from("MYKEY")).unwrap(), String::from("SATURN"));
        //In environ the value should be still 305
        assert_eq!(core.environ_get(&String::from("MYKEY")).unwrap(), String::from("305"));
    }

    fn create_tmpfile() -> tempfile::NamedTempFile {
        tempfile::NamedTempFile::new().unwrap()
    }

    fn create_tmpdir() -> tempfile::TempDir {
        tempfile::TempDir::new().unwrap()
    }
}
