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

use crate::{ShellCore, ShellState, ParseStatement};
use crate::process;
use std::collections::HashMap;
use dirs::home_dir;
use std::path::PathBuf;
use std::time::{Duration, Instant};

//Data types

/// ## Redirect
/// 
/// Redirect enum describes the redirect type of a command
#[derive(Clone, PartialEq, std::fmt::Debug)]
enum Redirect {
    Stdout,
    File(String)
}

impl ShellCore {

    /// ## new
    /// 
    /// Instantiate a new ShellCore
    pub fn new(wrkdir: PathBuf, history_size: usize, parser: Box<dyn ParseStatement>) -> ShellCore {
        let hostname: String = whoami::host();
        let username: String = whoami::username();
        let home: PathBuf = match home_dir() {
            Some(path) => PathBuf::from(path),
            None => PathBuf::from("/"),
        };
        ShellCore {
            state: ShellState::Idle,
            exit_code: 0,
            execution_time: Duration::from_millis(0),
            pid: None,
            wrk_dir: wrkdir,
            user: username,
            hostname: hostname,
            home_dir: home,
            prev_dir: home,
            execution_started: Instant::now(),
            storage: HashMap::new(),
            history: Vec::with_capacity(history_size),
            parser: parser,
            buf_in: String::new()
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    //fn test_core_new() {
    //    let core: ShellCore = ShellCore::new(PathBuf::from("/tmp/"), 2048, );
    //}
}