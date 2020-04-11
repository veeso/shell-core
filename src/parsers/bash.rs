//! # Bash
//!
//! `bash` is the Bash shell parser

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

extern crate getopts;

use crate::{HistoryOptions, ParseStatement, ParserError, ParserErrorCode, ShellCore, ShellExpression, ShellStatement, TaskRelation};
use getopts::Options;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::PathBuf;

pub struct Bash {}

/// ## BashParserState
/// 
/// Bash parser state describes the current parser state during the parsing of a bash script.
/// The states are stacked in the states attribute. It is not possible to finish a state unless it's on the top of the stack
#[derive(std::fmt::Debug)]
struct BashParserState {
    states: Vec<BashParserBlock>, //The States stack
    previous_char: char,
}

/// ### BashParserBlock
/// 
/// Bash Parser Block describes the bash block code type
#[derive(Clone, PartialEq, std::fmt::Debug)]
enum BashParserBlock {
    Escaped,
    Expression(char),   //Character which has been used to start the expression
    ForLoop,
    WhileLoop,
    Quoted(char)        //Character which has been used to start quoted
}

/// ## BashCodeBlock
/// 
/// BashCodeBlock describes the code block type in Bash
#[derive(Clone, PartialEq, std::fmt::Debug)]
enum BashCodeBlock {
    Case,
    For,
    Function,
    If,
    While
}

impl ParseStatement for Bash {
    fn parse(&self, core: &ShellCore, statement: &String) -> Result<ShellExpression, ParserError> {
        //Instantiate BashParserState
        let mut state: BashParserState = BashParserState::new();
        let mut argv: VecDeque<String> = match self.readline(statement) {
            Ok(argv) => argv,
            Err(err) => return Err(err)
        };
        self.parse_argv(core, state, argv)
    }
}

impl Bash {

    /// ### new
    /// 
    /// Instantiates a new Bash
    pub fn new() -> Bash {
        Bash {}
    }

    /// ### parse_argv
    /// 
    /// Recursive function which parse arguments and evaluates them into a ShellExpression
    fn parse_argv(&self, core: &ShellCore, mut state: BashParserState, mut input: VecDeque<String>) -> Result<ShellExpression, ParserError> {
        //Start iterating
        let mut statements: Vec<(ShellStatement, TaskRelation)> = Vec::new();
        loop {
            //TODO: impl
            //TODO: args becomes with '!', '$', '`'
        }
    }

    /// ### eval_expression
    /// 
    /// Evaluates an expression argument into a shell expression
    fn eval_expression(&self, core: &ShellCore, expression: &String) -> Result<ShellExpression, ParserError> {
        //Instantiate BashParserState
        let mut state: BashParserState = BashParserState::new();
        let mut argv: VecDeque<String> = match self.readline(expression) {
            Ok(argv) => argv,
            Err(err) => return Err(err)
        };
        self.parse_argv(core, state, argv)
    }

    /// ### readline
    /// 
    /// Get arguments from input string
    fn readline(&self, input: &String) -> Result<VecDeque<String>, ParserError> {
        let mut argv: VecDeque<String> = VecDeque::new();
        let mut states: BashParserState = BashParserState::new();
        let mut buffer: Vec<String> = Vec::new();
        //Split by word
        let mut index: usize = 0;
        for (row, line) in input.split("\n").enumerate() { //Iter over lines
            if row > 0 && line.len() > 0 {
                //Newlines are pushed as semicolon
                argv.push_back(String::from(";"));
            }
            for word in line.split_whitespace() { //Iter over word in lines
                let mut word_buf: String = String::with_capacity(word.len());
                //Iterate over word
                for (i, c) in word.chars().enumerate() {
                    let next: char = word.chars().nth(i + 1).unwrap_or(' ');
                    let mut skip_char: bool = false;
                    let prev_char: char = states.previous_char;
                    //If quote block is closed
                    if ((states.is_on_top(BashParserBlock::Quoted('"')) && c == '"') || (states.is_on_top(BashParserBlock::Quoted('\'')) && c == '\'')) && !states.is_escaped() {
                        skip_char = true;
                    }
                    if let Some(err) = states.update_state(c) {
                        return Err(ParserError::new(err, String::from(format!("bash: error at {}", index))))
                    }
                    index += 1;
                    //If quote block is opened, ignore char
                    if ((states.is_on_top(BashParserBlock::Quoted('"')) && c == '"') || (states.is_on_top(BashParserBlock::Quoted('\'')) && c == '\'')) && prev_char != '\\' {
                        skip_char = true;
                    }
                    //Ignore escape block opener
                    if states.is_escaped() && c == '\\' && next == '"' {
                        skip_char = true;
                    }
                    if ! skip_char {
                        word_buf.push(c);
                    }
                }
                //Push word buffer if big enough
                if word_buf.len() > 0 {
                    //Iter over word buffer and split some tokens
                    let mut remainder: Option<char> = None;
                    let mut escaped: bool = false;
                    let orig_word_buf = word_buf.clone();
                    for (index, c) in orig_word_buf.chars().enumerate() {
                        let next: Option<char> = orig_word_buf.chars().nth(index + 1);
                        if let Some(prev) = remainder {
                            //Look for &&
                            if prev == '&' && c == '&' {
                                let prev_part: String = String::from(&word_buf[..index - 1]);
                                if prev_part.len() > 0 {
                                    argv.push_back(prev_part); //Push word before &&
                                }
                                //Push &&
                                argv.push_back(String::from("&&"));
                                //Word buf becomes from the first char after &&
                                word_buf = String::from(&word_buf[index + 1..]);
                            } else if prev == '|' && c == '|' {
                                let prev_part: String = String::from(&word_buf[..index - 1]);
                                if prev_part.len() > 0 {
                                    argv.push_back(prev_part); //Push word before ||
                                }
                                //Push ||
                                argv.push_back(String::from("||"));
                                //Word buf becomes from the first char after ||
                                word_buf = String::from(&word_buf[index + 1..]);
                            } else if prev == '>' && c == '>' {
                                let prev_part: String = String::from(&word_buf[..index - 1]);
                                if prev_part.len() > 0 {
                                    argv.push_back(prev_part); //Push word before >>
                                }
                                //Push >>
                                argv.push_back(String::from(">>"));
                                //Word buf becomes from the first char after >>
                                word_buf = String::from(&word_buf[index + 1..]);
                            } else if prev == '<' && c == '<' {
                                let prev_part: String = String::from(&word_buf[..index - 1]);
                                if prev_part.len() > 0 {
                                    argv.push_back(prev_part); //Push word before <<
                                }
                                //Push <<
                                argv.push_back(String::from("<<"));
                                //Word buf becomes from the first char after <<
                                word_buf = String::from(&word_buf[index + 1..]);
                            }
                        } else if ! escaped {
                            //Check if pipe
                            if let Some(next) = next {
                                if c == '|' && next != '|' {
                                    let prev_part: String = String::from(&word_buf[..index]);
                                    if prev_part.len() > 0 {
                                        argv.push_back(prev_part); //Push word before |
                                    }
                                    //Push &&
                                    argv.push_back(String::from("|"));
                                    //Word buf becomes from the first char after |
                                    word_buf = String::from(&word_buf[index + 1..]);
                                } else if c == '|' && next == '|' {
                                    //Set remainder
                                    remainder = Some('|');
                                } else if c == '>' && next != '>' {
                                    let prev_part: String = String::from(&word_buf[..index]);
                                    if prev_part.len() > 0 {
                                        argv.push_back(prev_part); //Push word before >
                                    }
                                    //Push >
                                    argv.push_back(String::from(">"));
                                    //Word buf becomes from the first char after >
                                    word_buf = String::from(&word_buf[index + 1..]);
                                } else if c == '>' && next == '>' {
                                    //Set remainder
                                    remainder = Some('>');
                                } else if c == '<' && next != '<' {
                                    let prev_part: String = String::from(&word_buf[..index]);
                                    if prev_part.len() > 0 {
                                        argv.push_back(prev_part); //Push word before <
                                    }
                                    //Push >
                                    argv.push_back(String::from("<"));
                                    //Word buf becomes from the first char after <
                                    word_buf = String::from(&word_buf[index + 1..]);
                                } else if c == '<' && next == '<' {
                                    //Set remainder
                                    remainder = Some('<');
                                } else if c == '&' && next == '&' {
                                    //Set remainder
                                    remainder = Some('&');
                                } else if c == '&' && next != '&' {
                                    let prev_part: String = String::from(&word_buf[..index]);
                                    if prev_part.len() > 0 {
                                        argv.push_back(prev_part); //Push word before &
                                    }
                                    //Push &
                                    argv.push_back(String::from("&"));
                                    //Word buf becomes from the first char after &
                                    word_buf = String::from(&word_buf[index + 1..]);
                                }
                            }
                            //Handle semicolon
                            if c == ';' {
                                let prev_part: String = String::from(&word_buf[..index]);
                                if prev_part.len() > 0 {
                                    argv.push_back(prev_part); //Push word before ;
                                }
                                //Push ;
                                argv.push_back(String::from(";"));
                                //Word buf becomes from the first char after ;
                                word_buf = String::from(&word_buf[index + 1..]);
                            }
                        }
                        if c == '\\' {
                            escaped = true;
                        } else {
                            escaped = false;
                        }
                    }
                    //Push remaining word buffer
                    if word_buf.len() > 0 {
                        buffer.push(word_buf);
                    }
                }
                index += 1; //Increment cause of whitespace
                //If states stack is empty, push argument
                if states.empty() {
                    if buffer.len() > 0 {
                        argv.push_back(buffer.join(" "));
                        buffer.clear();
                    }
                }
            } //End of line
        }
        Ok(argv)
    }

    /// ### is_ligature
    /// 
    /// Returns whether the next token is a ligature
    fn is_ligature(&self, arg: &String) -> bool {
        if arg == "&&" {
            true
        } else if arg == "||" {
            true
        } else if arg == "|" {
            true
        } else if arg == ";" {
            true
        } else if arg == "&" {
            true
        } else if arg == "&&" {
            true
        } else if arg == ">" {
            true
        } else if arg == ">>" {
            true
        } else if arg == "<" {
            true
        } else if arg == "<<" {
            true
        } else {
            false
        }
    }

    /// ### cut_argv_to_delim
    /// 
    /// Cut arguments until the first delimiter is found
    fn cut_argv_to_delim(&self, argv: &mut VecDeque<String>) {
        let mut trunc: usize = 0;
        for arg in argv.iter() {
            if self.is_ligature(&arg) {
                break;
            } else {
                trunc += 1;
            }
        }
        for i in 0..trunc {
            argv.pop_front();
        } 
    }

    //@! Statements parsers

    /// ### parse_alias
    /// 
    /// Parse alias arguments
    fn parse_alias(&self, core: &ShellCore, argv: &mut VecDeque<String>) -> Result<ShellStatement, ParserError> {
        /*
            Alias has three cases
            - alias_name => Returns the alias value
            - alias_name=alias_value => set name to value
            - no arguments => Returns all the aliases
        */
        let mut alias_name: Option<String> = None;
        let mut alias_value: Option<String> = None;
        if argv.len() > 0 {
            //Get first argument
            let arg: String = argv.get(0).unwrap().to_string();
            if ! self.is_ligature(&arg) { //If arg is not ligature, Treat arg 0
                let mut buff: String = String::new();
                let mut escaped: bool = false;
                //Iterate over argument characters
                for c in arg.chars() {
                    if ! escaped { //Handle separators and other stuff
                        if c == '=' && alias_name.is_none() { //Value starts
                            alias_name = Some(buff.clone());
                            buff.clear();
                            continue;
                        }
                    }
                    //Handle escape
                    if c == '\\' && ! escaped {
                        escaped = true;
                    } else {
                        escaped = false;
                    }
                    //Push character to buff
                    buff.push(c);
                }
                if alias_name.is_some() {
                    //Set value
                    alias_value = Some(buff.clone());
                } else {
                    alias_name = Some(buff.clone());
                }
            }
        }
        //Remove useless arguments
        self.cut_argv_to_delim(argv);
        //Return Alias Shell Statement
        Ok(ShellStatement::Alias(alias_name, alias_value))
    }

    //TODO: case
    //TODO: command

    /// ### parse_cd
    /// 
    /// Parse CD statement. Returns the ShellStatement parsed.
    /// Cd is already removed from input
    fn parse_cd(&self, core: &ShellCore, argv: &mut VecDeque<String>) -> Result<ShellStatement, ParserError> {
        //If dir is none, return get home or buffer, otherwise resolve path
        let dir: PathBuf = match argv.len() {
            0 => core.get_home(),
            _ => {
                //Check if second argument is ligature
                if argv.len() > 1 {
                    if ! self.is_ligature(&argv[1]) {
                        self.cut_argv_to_delim(argv);
                        return Err(ParserError::new(ParserErrorCode::TooManyArgs, String::from("bash: cd: too many arguments")))
                    }
                }
                if self.is_ligature(&argv[0]) {
                    core.get_home()
                } else {
                    let first_arg: String = String::from(argv.front().unwrap().as_str().trim());
                    //Return home/prev/path
                    match first_arg.as_str() {
                        "~" => core.get_home(),
                        "-" => core.get_prev_dir(),
                        _ => PathBuf::from(first_arg.as_str())
                    }
                }
            }
        };
        //Remove useless arguments
        self.cut_argv_to_delim(argv);
        //Return Cd statement
        Ok(ShellStatement::Cd(dir))
    }

    //TODO: declare
    
    /// ### parse_dirs
    /// 
    /// Parse dirs arguments
    fn parse_dirs(&self, argv: &mut VecDeque<String>) -> Result<ShellStatement, ParserError> {
        //Check args
        if argv.len() > 0 {
            let arg: String = argv.get(0).unwrap().clone();
            if ! self.is_ligature(&arg) {
                //Too many arguments
                self.cut_argv_to_delim(argv);
                return Err(ParserError::new(ParserErrorCode::TooManyArgs, format!("bash: dirs: {}: invalid option", arg)))
            }
        }
        //Remove useless arguments
        self.cut_argv_to_delim(argv);
        Ok(ShellStatement::Dirs)
    }

    //TODO: exec

    /// ### parse_exit
    /// 
    /// Parse exit arguments
    fn parse_exit(&self, argv: &mut VecDeque<String>) -> Result<ShellStatement, ParserError> {
        let mut exitcode: Option<u8> = None;
        if argv.len() > 0 {
            let arg: &String = argv.get(0).unwrap();
            if ! self.is_ligature(arg) {
                exitcode = Some(arg.parse::<u8>().unwrap_or(2));
            }
        }
        //Remove arguments
        self.cut_argv_to_delim(argv);
        Ok(ShellStatement::Exit(exitcode.unwrap_or(0)))
    }

    /// ### parse_export
    /// 
    /// Parse export arguments
    fn parse_export(&self, core: &ShellCore, argv: &mut VecDeque<String>) -> Result<ShellStatement, ParserError> {
        let mut cmdarg: Vec<String> = Vec::new();
        for arg in argv.iter() {
            if ! self.is_ligature(&arg) {
                cmdarg.push(arg.to_string());
            } else {
                break;
            }
        }
        //Remove useless arguments
        self.cut_argv_to_delim(argv);
        //Parse cmdarg
        let mut opts = Options::new();
        opts.optflag("p", "print", "Print all exported variables");
        opts.optflag("h", "help", "Display help");
        let matches = match opts.parse(&cmdarg) {
            Ok(m) => m,
            Err(e) => {
                return Ok(ShellStatement::Output(None, Some(String::from(format!("bash: Export invalid option: {}", e.to_string())))))
            }
        };
        //Handle help
        if matches.opt_present("h") {
            return Ok(ShellStatement::Output(Some(opts.usage("export")), None))
        }
        //Handle print
        if matches.opt_present("p") {
            //Retrieve all values from environ
            let environ: HashMap<String, String> = core.environ_getall();
            let mut output: String = String::new();
            for (key, value) in environ.iter() {
                output.push_str(format!("declare -x {}={}\n", key, value).as_str());
            }
            Ok(ShellStatement::Output(Some(output), None))
        } else {
            //Handle extra arguments
            let optarg: Vec<String> = matches.free.clone();
            if optarg.len() > 0 {
                let arg: String = optarg.get(0).unwrap().to_string();
                let mut key: String = String::new();
                let mut val: String = String::new();
                let mut buff: String = String::new();
                let mut escaped: bool = false;
                //Iterate over argument characters
                for c in arg.chars() {
                    if ! escaped { //Handle separators and other stuff
                        if c == '=' && ! key.is_empty() { //Value starts
                            key = buff.clone();
                            buff.clear();
                            continue;
                        }
                    }
                    //Handle escape
                    if c == '\\' && ! escaped {
                        escaped = true;
                    } else {
                        escaped = false;
                    }
                    //Push character to buff
                    buff.push(c);
                }
                if ! key.is_empty() {
                    //Set value
                    val = buff.clone();
                } else {
                    key = buff.clone();
                }
                //Evaluate value as an expression
                let val: ShellExpression = match self.eval_expression(core, &val) {
                    Ok(expr) => expr,
                    Err(err) => return Err(err)
                };
                //Return export
                Ok(ShellStatement::Export(key, val))
            } else { //No args
                //Display all
                //Retrieve all values from environ
                let environ: HashMap<String, String> = core.environ_getall();
                let mut output: String = String::new();
                for (key, value) in environ.iter() {
                    output.push_str(format!("declare -x {}={}\n", key, value).as_str());
                }
                Ok(ShellStatement::Output(Some(output), None))
            }
        }
    }

    //TODO: for
    //TODO: function
    //TODO: getopts
    //TODO: help
    
    /// ### parse_history
    /// 
    /// Parse history command arguments
    fn parse_history(&self, core: &ShellCore, argv: &mut VecDeque<String>) -> Result<ShellStatement, ParserError> {
        let mut cmdarg: Vec<String> = Vec::new();
        for arg in argv.iter() {
            if ! self.is_ligature(&arg) {
                cmdarg.push(arg.to_string());
            } else {
                break;
            }
        }
        //Remove useless arguments
        self.cut_argv_to_delim(argv);
        //Parse cmdarg
        let mut opts = Options::new();
        opts.optopt("a", "", "Append the new history lines to the history file", "<file>");
        opts.optflag("c", "", "Clear the history list. This may be combined with the other options to replace the history list completely.");
        opts.optopt("d", "", "Delete the history entry at position offset", "<offset>");
        opts.optopt("r", "", "Read the history file and append its contents to the history list.", "<file>");
        opts.optopt("w", "", "Write out the current history list to the history file.", "<file>");
        opts.optflag("h", "help", "Display help");
        let matches = match opts.parse(&cmdarg) {
            Ok(m) => m,
            Err(e) => {
                return Ok(ShellStatement::Output(None, Some(String::from(format!("bash: History invalid option: {}", e.to_string())))))
            }
        };
        //Handle help
        if matches.opt_present("h") {
            Ok(ShellStatement::Output(Some(opts.usage("history")), None))
        } else if let Some(file) = matches.opt_str("a") {
            //Resolve path
            let file: String = core.resolve_path(file.clone()).into_os_string().into_string().unwrap_or(file);
            //Append history to file
            Ok(ShellStatement::History(HistoryOptions::Write(file, false)))
        } else if matches.opt_present("c") {
            //Clear history
            Ok(ShellStatement::History(HistoryOptions::Clear))
        } else if let Some(index) = matches.opt_str("d") {
            //Split history and return rc
            if let Ok(index) = index.parse::<usize>() {
                Ok(ShellStatement::History(HistoryOptions::Del(index)))
            } else {
                Err(ParserError::new(ParserErrorCode::TooManyArgs, String::from("history del index must be a number")))
            }
        } else if let Some(file) = matches.opt_str("r") {
            //Resolve path
            let file: String = core.resolve_path(file.clone()).into_os_string().into_string().unwrap_or(file);
            //Read history
            Ok(ShellStatement::History(HistoryOptions::Read(file)))
        } else if let Some(file) = matches.opt_str("w") {
            //Resolve path
            let file: String = core.resolve_path(file.clone()).into_os_string().into_string().unwrap_or(file);
            //Write file (truncate)
            Ok(ShellStatement::History(HistoryOptions::Write(file, true)))
        } else {
            //Print history
            Ok(ShellStatement::History(HistoryOptions::Print))
        }
    }
    //TODO: if
    //TODO: let
    //TODO: local
    //TODO: logout
    //TODO: popd
    //TODO: pushd
    //TODO: read
    
    /// ### parse_return
    /// 
    /// Parse return arguments
    fn parse_return(&self, argv: &mut VecDeque<String>) -> Result<ShellStatement, ParserError> {
        let mut exitcode: Option<u8> = None;
        if argv.len() > 0 {
            let arg: &String = argv.get(0).unwrap();
            if ! self.is_ligature(arg) {
                exitcode = Some(arg.parse::<u8>().unwrap_or(2));
            }
        }
        //Remove arguments
        self.cut_argv_to_delim(argv);
        Ok(ShellStatement::Return(exitcode.unwrap_or(0)))
    }
    //TODO: set
    //TODO: source
    //TODO: time
    //TODO: until/while
    
}

//@! Structs

impl BashParserState {

    /// ### new
    /// 
    /// Instantiates a new BashParserState
    pub(crate) fn new() -> BashParserState {
        BashParserState {
            states: Vec::new(),
            previous_char: ' '
        }
    }

    /// ### update_state
    /// 
    /// Update current state based on last character.
    /// In case of errors, a Parser Error is returned
    pub(crate) fn update_state(&mut self, ch: char) -> Option<ParserErrorCode> {
        //If Char is backslash
        if ch == '\\' {
            if self.is_on_top(BashParserBlock::Escaped) { //If was escaped, pop escape
                self.pop();
            } else { //Otherwise becomes escaped
                self.stack_state(BashParserBlock::Escaped);
            }
        }
        if ! self.is_on_top(BashParserBlock::Escaped) {
            if ch == '"' { //Quotes
                if self.is_on_top(BashParserBlock::Quoted('"')) {
                    //Quote terminates
                    self.pop();
                } else if self.is_on_top(BashParserBlock::Quoted('\'')) {
                    //Ignore
                } else {
                    //Set quoted
                    self.stack_state(BashParserBlock::Quoted('"'));
                }
            } else if ch == '\'' { //Quotes
                if self.is_on_top(BashParserBlock::Quoted('\'')) {
                    //Quote terminates
                    self.pop();
                } else if self.is_on_top(BashParserBlock::Quoted('"')) {
                    //Ignore
                } else {
                    //Set quoted
                    self.stack_state(BashParserBlock::Quoted('\''));
                }
            } else if ! self.is_quoted() { //If not quoted, try expressions
                if ch == '(' && self.previous_char == '$' { //Expression open and not quoted and If previous character is '$'
                    //Start expression
                    self.stack_state(BashParserBlock::Expression('('));
                } else if ch == '`' { //Expression open/close
                    //If not in expression
                    if self.is_on_top(BashParserBlock::Expression('`')) {
                        self.pop(); //Terminate expression
                    } else { //Else stack state
                        self.stack_state(BashParserBlock::Expression('`'));
                    }
                } else if ch == ')' {
                    //If not in expression of that kind, return error
                    if self.is_on_top(BashParserBlock::Expression('(')) {
                        self.pop(); //Pop
                    } else {
                        //Return bad token (tried to close an expression that wasn't opened, or before another token)
                        return Some(ParserErrorCode::BadToken)
                    }
                }
            }
        }
        if self.is_escaped() && ch != '\\' { //If was escaped, pop escape
            self.pop();
        }
        //Eventually update previous ch
        self.previous_char = ch;
        None
    }

    /// ### empty
    /// 
    /// Returns whether the states stack is empty
    pub(crate) fn empty(&self) -> bool {
        self.states.len() == 0
    }

    /// ### is_quoted
    /// 
    /// Returns whether is quoted
    pub(crate) fn is_quoted(&self) -> bool {
        self.is_on_top(BashParserBlock::Quoted('"')) || self.is_on_top(BashParserBlock::Quoted('\''))
    }

    /// ### is_escaped
    /// 
    /// Returns whether is escaped
    pub(crate) fn is_escaped(&self) -> bool {
        self.is_on_top(BashParserBlock::Escaped)
    }

    /// ### is_inside_expression
    /// 
    /// Returns whether is inside an expression
    pub(crate) fn is_inside_expression(&self) -> bool {
        self.is_on_top(BashParserBlock::Expression('(')) || self.is_on_top(BashParserBlock::Expression('`'))
    }

    /// ### is_in_for_loop
    /// 
    /// Returns whether is inside a for loop
    pub(crate) fn is_in_for_loop(&self) -> bool {
        self.is_on_top(BashParserBlock::ForLoop)
    }

    /// ### open_for_loop
    /// 
    /// Open a for loop, pushing it on the top of the stack
    pub(crate) fn open_for_loop(&mut self) {
        self.stack_state(BashParserBlock::ForLoop);
    }

    /// ### close_for_loop
    /// 
    /// Close a for loop, popping from its top the for loop
    pub(crate) fn close_for_loop(&mut self) -> Result<(), ParserErrorCode> {
        if self.is_in_for_loop() {
            self.pop();
            Ok(())
        } else {
            Err(ParserErrorCode::BadToken)
        }
    }

    /// ### is_in_for_while
    /// 
    /// Returns whether is inside a while loop
    pub(crate) fn is_in_while_loop(&self) -> bool {
        self.is_on_top(BashParserBlock::WhileLoop)
    }

    /// ### open_while_loop
    /// 
    /// Open a while loop, pushing it on the top of the stack
    pub(crate) fn open_while_loop(&mut self) {
        self.stack_state(BashParserBlock::WhileLoop);
    }

    /// ### close_for_loop
    /// 
    /// Close a while loop, popping from its top the while loop
    pub(crate) fn close_while_loop(&mut self) -> Result<(), ParserErrorCode> {
        if self.is_in_while_loop() {
            self.pop();
            Ok(())
        } else {
            Err(ParserErrorCode::BadToken)
        }
    }

    /// ### is_on_top
    /// 
    /// Verifies if the provided state is currently on the top
    fn is_on_top(&self, state: BashParserBlock) -> bool {
        match self.states.last() {
            None => false,
            Some(s) => match s {
                BashParserBlock::Escaped => {
                    if let BashParserBlock::Escaped = state {
                        true
                    } else {
                        false
                    }
                },
                BashParserBlock::Expression(ex) => {
                    if let BashParserBlock::Expression(s_expr) = state {
                        *ex == s_expr
                    } else {
                        false
                    }
                },
                BashParserBlock::ForLoop => {
                    if let BashParserBlock::ForLoop = state {
                        true
                    } else {
                        false
                    }
                },
                BashParserBlock::Quoted(q) => {
                    if let BashParserBlock::Quoted(s_quoted) = state {
                        *q == s_quoted
                    } else {
                        false
                    }
                },
                BashParserBlock::WhileLoop => {
                    if let BashParserBlock::WhileLoop = state {
                        true
                    } else {
                        false
                    }
                }
            }
        }
    }

    /// ### stack_state
    /// 
    /// Stack a new state on the top of the stack
    fn stack_state(&mut self, state: BashParserBlock) {
        self.states.push(state);
    }

    /// ### pop
    /// 
    /// Pop a state from the stack
    fn pop(&mut self) {
        self.states.pop();
    }

}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::ShellCore;
    use crate::UserStream;

    #[test]
    fn test_bash_parser_state_new() {
        let parser_state: BashParserState = BashParserState::new();
        assert_eq!(parser_state.states.len(), 0);
        assert!(parser_state.empty());
        assert_eq!(parser_state.previous_char, ' ');
    }

    #[test]
    fn test_bash_parser_readline() {
        let parser: Bash = Bash::new();
        assert_eq!(parser.readline(&String::from("cd /tmp/")).unwrap(), vec![String::from("cd"), String::from("/tmp/")]);
        assert_eq!(parser.readline(&String::from("cd;")).unwrap(), vec![String::from("cd"), String::from(";")]);
        assert_eq!(parser.readline(&String::from("echo \"foo bar\"")).unwrap(), vec![String::from("echo"), String::from("foo bar")]);
        assert_eq!(parser.readline(&String::from("echo \"'foo' 'bar'\"")).unwrap(), vec![String::from("echo"), String::from("'foo' 'bar'")]);
        assert_eq!(parser.readline(&String::from("echo \"\\\"foo bar\\\"\"")).unwrap(), vec![String::from("echo"), String::from("\"foo bar\"")]);
        //Escapes
        assert_eq!(parser.readline(&String::from("cd \\;")).unwrap(), vec![String::from("cd"), String::from("\\;")]);
        //Try error
        assert!(parser.readline(&String::from("echo \"$(pw\"d)")).is_err());
        //Over lines
        assert_eq!(parser.readline(&String::from("cd /tmp/\ncd /home/")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from(";"), String::from("cd"), String::from("/home/")]);
        //Separators (&&)
        assert_eq!(parser.readline(&String::from("cd /tmp/ && exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from("&&"), String::from("exit")]);
        assert_eq!(parser.readline(&String::from("cd /tmp/ &&exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from("&&"), String::from("exit")]);
        assert_eq!(parser.readline(&String::from("cd /tmp/&&exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from("&&"), String::from("exit")]);
        //Separators (||)
        assert_eq!(parser.readline(&String::from("cd /tmp/ || exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from("||"), String::from("exit")]);
        assert_eq!(parser.readline(&String::from("cd /tmp/ ||exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from("||"), String::from("exit")]);
        assert_eq!(parser.readline(&String::from("cd /tmp/||exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from("||"), String::from("exit")]);
        //Separators (|)
        assert_eq!(parser.readline(&String::from("cd /tmp/ | exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from("|"), String::from("exit")]);
        assert_eq!(parser.readline(&String::from("cd /tmp/ |exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from("|"), String::from("exit")]);
        assert_eq!(parser.readline(&String::from("cd /tmp/|exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from("|"), String::from("exit")]);
        //Separators (;)
        assert_eq!(parser.readline(&String::from("cd /tmp/ ; exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from(";"), String::from("exit")]);
        assert_eq!(parser.readline(&String::from("cd /tmp/ ;exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from(";"), String::from("exit")]);
        assert_eq!(parser.readline(&String::from("cd /tmp/;exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from(";"), String::from("exit")]);
        //Separators (>>)
        assert_eq!(parser.readline(&String::from("cd /tmp/ >> exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from(">>"), String::from("exit")]);
        assert_eq!(parser.readline(&String::from("cd /tmp/ >>exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from(">>"), String::from("exit")]);
        assert_eq!(parser.readline(&String::from("cd /tmp/>>exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from(">>"), String::from("exit")]);
        //Separators (<<)
        assert_eq!(parser.readline(&String::from("cd /tmp/ << exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from("<<"), String::from("exit")]);
        assert_eq!(parser.readline(&String::from("cd /tmp/ <<exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from("<<"), String::from("exit")]);
        assert_eq!(parser.readline(&String::from("cd /tmp/<<exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from("<<"), String::from("exit")]);
        //Separators (>)
        assert_eq!(parser.readline(&String::from("cd /tmp/ > exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from(">"), String::from("exit")]);
        assert_eq!(parser.readline(&String::from("cd /tmp/ >exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from(">"), String::from("exit")]);
        assert_eq!(parser.readline(&String::from("cd /tmp/>exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from(">"), String::from("exit")]);
        //Separators (<)
        assert_eq!(parser.readline(&String::from("cd /tmp/ < exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from("<"), String::from("exit")]);
        assert_eq!(parser.readline(&String::from("cd /tmp/ <exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from("<"), String::from("exit")]);
        assert_eq!(parser.readline(&String::from("cd /tmp/<exit")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from("<"), String::from("exit")]);
    }

    #[test]
    fn test_bash_parser_alias() {
        let (core, _): (ShellCore, UserStream) = ShellCore::new(None, 32, Box::new(Bash::new()));
        let parser: Bash = Bash::new();
        //Simple set case
        let mut input: VecDeque<String> = parser.readline(&String::from("l=ls")).unwrap();
        assert_eq!(parser.parse_alias(&core, &mut input).unwrap(), ShellStatement::Alias(Some(String::from("l")), Some(String::from("ls"))));
        assert_eq!(input.len(), 0); //Should be empty
        //Simple set case with quote
        let mut input: VecDeque<String> = parser.readline(&String::from("ll='ls -l'")).unwrap();
        assert_eq!(parser.parse_alias(&core, &mut input).unwrap(), ShellStatement::Alias(Some(String::from("ll")), Some(String::from("ls -l"))));
        assert_eq!(input.len(), 0); //Should be empty
        let mut input: VecDeque<String> = parser.readline(&String::from("ll=\"ls -l\"")).unwrap();
        assert_eq!(parser.parse_alias(&core, &mut input).unwrap(), ShellStatement::Alias(Some(String::from("ll")), Some(String::from("ls -l"))));
        assert_eq!(input.len(), 0); //Should be empty
        //Set case with escapes
        let mut input: VecDeque<String> = parser.readline(&String::from("noise='echo \"ZZZ\"'")).unwrap();
        assert_eq!(parser.parse_alias(&core, &mut input).unwrap(), ShellStatement::Alias(Some(String::from("noise")), Some(String::from("echo \"ZZZ\""))));
        assert_eq!(input.len(), 0); //Should be empty
        let mut input: VecDeque<String> = parser.readline(&String::from("noise='echo \\'ZZZ\\''")).unwrap();
        assert_eq!(parser.parse_alias(&core, &mut input).unwrap(), ShellStatement::Alias(Some(String::from("noise")), Some(String::from("echo \\'ZZZ\\'"))));
        assert_eq!(input.len(), 0); //Should be empty
        //Alias getter
        let mut input: VecDeque<String> = parser.readline(&String::from("ll")).unwrap();
        assert_eq!(parser.parse_alias(&core, &mut input).unwrap(), ShellStatement::Alias(Some(String::from("ll")), None));
        assert_eq!(input.len(), 0); //Should be empty
        //Alias get all
        let mut input: VecDeque<String> = VecDeque::new();
        assert_eq!(parser.parse_alias(&core, &mut input).unwrap(), ShellStatement::Alias(None, None));
        assert_eq!(input.len(), 0); //Should be empty
    }

    #[test]
    fn test_bash_parser_cd() {
        let (core, _): (ShellCore, UserStream) = ShellCore::new(None, 32, Box::new(Bash::new()));
        let parser: Bash = Bash::new();
        //Parse some CD statements
        //Simple case
        let mut input: VecDeque<String> = parser.readline(&String::from("/tmp")).unwrap();
        assert_eq!(parser.parse_cd(&core, &mut input).unwrap(), ShellStatement::Cd(PathBuf::from("/tmp")));
        assert_eq!(input.len(), 0); //Should be empty
        //With semicolon
        let mut input: VecDeque<String> = parser.readline(&String::from("/tmp;")).unwrap();
        assert_eq!(parser.parse_cd(&core, &mut input).unwrap(), ShellStatement::Cd(PathBuf::from("/tmp")));
        assert_eq!(input, vec![String::from(";")]); //Should be empty
        //With newline
        let mut input: VecDeque<String> = parser.readline(&String::from("/tmp\n")).unwrap();
        assert_eq!(parser.parse_cd(&core, &mut input).unwrap(), ShellStatement::Cd(PathBuf::from("/tmp")));
        assert_eq!(input.len(), 0); //Should be empty
        //Too many arguments
        let mut input: VecDeque<String> = parser.readline(&String::from("/tmp /home/")).unwrap();
        assert!(parser.parse_cd(&core, &mut input).is_err());
        assert_eq!(input.len(), 0); //Should be empty
        //Too many arguments 2
        let mut input: VecDeque<String> = parser.readline(&String::from("/tmp /home/;")).unwrap();
        assert!(parser.parse_cd(&core, &mut input).is_err());
        assert_eq!(input, vec![String::from(";")]); //Should be empty
        //False too many arguments
        let mut input: VecDeque<String> = parser.readline(&String::from("/tmp ;")).unwrap();
        assert_eq!(parser.parse_cd(&core, &mut input).unwrap(), ShellStatement::Cd(PathBuf::from("/tmp")));
        assert_eq!(input, vec![String::from(";")]); //Should be empty
        //Too many arguments due to escape
        let mut input: VecDeque<String> = parser.readline(&String::from("/tmp \\;")).unwrap();
        assert!(parser.parse_cd(&core, &mut input).is_err());
        assert_eq!(input.len(), 0); //Should be empty
        //Quotes
        let mut input: VecDeque<String> = parser.readline(&String::from("\"/home\"")).unwrap();
        assert_eq!(parser.parse_cd(&core, &mut input).unwrap(), ShellStatement::Cd(PathBuf::from("/home/")));
        assert_eq!(input.len(), 0); //Should be empty
        //Escaped quotes
        let mut input: VecDeque<String> = parser.readline(&String::from("/home/\\\"foo\\\"")).unwrap();
        assert_eq!(parser.parse_cd(&core, &mut input).unwrap(), ShellStatement::Cd(PathBuf::from("/home/\"foo\"")));
        assert_eq!(input.len(), 0); //Should be empty
        //With and
        let mut input: VecDeque<String> = parser.readline(&String::from("/tmp &&")).unwrap();
        assert_eq!(parser.parse_cd(&core, &mut input).unwrap(), ShellStatement::Cd(PathBuf::from("/tmp")));
        assert_eq!(input, vec![String::from("&&")]); //Should be &&
        //Special cases
        let mut input: VecDeque<String> = parser.readline(&String::from("~")).unwrap();
        assert_eq!(parser.parse_cd(&core, &mut input).unwrap(), ShellStatement::Cd(core.get_home()));
        assert_eq!(input.len(), 0); //Should be empty
        let mut input: VecDeque<String> = parser.readline(&String::from("-")).unwrap();
        assert_eq!(parser.parse_cd(&core, &mut input).unwrap(), ShellStatement::Cd(core.get_prev_dir()));
        assert_eq!(input.len(), 0); //Should be empty
    }

    #[test]
    fn test_bash_parser_dirs() {
        let (core, _): (ShellCore, UserStream) = ShellCore::new(None, 32, Box::new(Bash::new()));
        let parser: Bash = Bash::new();
        //Simple case
        let mut input: VecDeque<String> = parser.readline(&String::from("")).unwrap();
        assert_eq!(parser.parse_dirs(&mut input).unwrap(), ShellStatement::Dirs);
        assert_eq!(input.len(), 0); //Should be empty
        //Arg is ligature
        let mut input: VecDeque<String> = parser.readline(&String::from(";")).unwrap();
        assert_eq!(parser.parse_dirs(&mut input).unwrap(), ShellStatement::Dirs);
        assert_eq!(input.len(), 1); //Should be empty
        //Bad arg
        let mut input: VecDeque<String> = parser.readline(&String::from("a")).unwrap();
        assert!(parser.parse_dirs(&mut input).is_err());
        assert_eq!(input.len(), 0); //Should be empty
    }

    #[test]
    fn test_bash_parser_exit() {
        let (core, _): (ShellCore, UserStream) = ShellCore::new(None, 32, Box::new(Bash::new()));
        let parser: Bash = Bash::new();
        //Simple case
        let mut input: VecDeque<String> = parser.readline(&String::from("0")).unwrap();
        assert_eq!(parser.parse_exit(&mut input).unwrap(), ShellStatement::Exit(0));
        assert_eq!(input.len(), 0); //Should be empty
        //Simple case
        let mut input: VecDeque<String> = parser.readline(&String::from("128")).unwrap();
        assert_eq!(parser.parse_exit(&mut input).unwrap(), ShellStatement::Exit(128));
        assert_eq!(input.len(), 0); //Should be empty
        //Bad case
        let mut input: VecDeque<String> = parser.readline(&String::from("foobar")).unwrap();
        assert_eq!(parser.parse_exit(&mut input).unwrap(), ShellStatement::Exit(2));
        assert_eq!(input.len(), 0); //Should be empty
        //No arg
        let mut input: VecDeque<String> = VecDeque::new();
        assert_eq!(parser.parse_exit(&mut input).unwrap(), ShellStatement::Exit(0));
        assert_eq!(input.len(), 0); //Should be empty
    }

    #[test]
    fn test_bash_parser_export() {
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 32, Box::new(Bash::new()));
        let parser: Bash = Bash::new();
        //Export two values first
        core.environ_set(String::from("FOO"), String::from("1"));
        core.environ_set(String::from("BAR"), String::from("2"));
        //No args
        let mut input: VecDeque<String> = parser.readline(&String::from("")).unwrap();
        assert!(parser.parse_export(&core, &mut input).is_ok()); //Print environment, but it's too long to be compared, it's variable too
        assert_eq!(input.len(), 0); //Should be empty
        //Print argument
        let mut input: VecDeque<String> = parser.readline(&String::from("-p")).unwrap();
        assert!(parser.parse_export(&core, &mut input).is_ok()); //Print environment, but it's too long to be compared, it's variable too
        assert_eq!(input.len(), 0); //Should be empty
        //Help argument
        let mut input: VecDeque<String> = parser.readline(&String::from("-h")).unwrap();
        assert_eq!(parser.parse_export(&core, &mut input).unwrap(), ShellStatement::Output(Some(String::from("export\n\nOptions:\n    -p, --print         Print all exported variables\n    -h, --help          Display help\n")), None)); //Prints help
        assert_eq!(input.len(), 0); //Should be empty
        //TODO: parse_argv required for value assignation
    }

    #[test]
    fn test_bash_parser_history() {
        let (mut core, _): (ShellCore, UserStream) = ShellCore::new(None, 32, Box::new(Bash::new()));
        let parser: Bash = Bash::new();
        //No args
        let mut input: VecDeque<String> = parser.readline(&String::from("")).unwrap();
        assert_eq!(parser.parse_history(&core, &mut input).unwrap(), ShellStatement::History(HistoryOptions::Print));
        assert_eq!(input.len(), 0);
        //Append history
        let history: String = format!("{}/.bash_history", core.get_home().as_path().display());
        let mut input: VecDeque<String> = parser.readline(&String::from("-a ~/.bash_history")).unwrap();
        assert_eq!(parser.parse_history(&core, &mut input).unwrap(), ShellStatement::History(HistoryOptions::Write(history.clone(), false)));
        assert_eq!(input.len(), 0);
        //Clear history
        let mut input: VecDeque<String> = parser.readline(&String::from("-c")).unwrap();
        assert_eq!(parser.parse_history(&core, &mut input).unwrap(), ShellStatement::History(HistoryOptions::Clear));
        assert_eq!(input.len(), 0);
        //Delete history index
        let mut input: VecDeque<String> = parser.readline(&String::from("-d 42")).unwrap();
        assert_eq!(parser.parse_history(&core, &mut input).unwrap(), ShellStatement::History(HistoryOptions::Del(42)));
        assert_eq!(input.len(), 0);
        //Read the history
        let mut input: VecDeque<String> = parser.readline(&String::from("-r ~/.bash_history")).unwrap();
        assert_eq!(parser.parse_history(&core, &mut input).unwrap(), ShellStatement::History(HistoryOptions::Read(history.clone())));
        assert_eq!(input.len(), 0);
        //Write history
        let mut input: VecDeque<String> = parser.readline(&String::from("-w ~/.bash_history")).unwrap();
        assert_eq!(parser.parse_history(&core, &mut input).unwrap(), ShellStatement::History(HistoryOptions::Write(history.clone(), true)));
        assert_eq!(input.len(), 0);
        //Help
        let mut input: VecDeque<String> = parser.readline(&String::from("-h")).unwrap();
        assert_eq!(parser.parse_history(&core, &mut input).unwrap(), ShellStatement::Output(Some(String::from("history\n\nOptions:\n    -a <file>           Append the new history lines to the history file\n    -c                  Clear the history list. This may be combined with the\n                        other options to replace the history list completely.\n    -d <offset>         Delete the history entry at position offset\n    -r <file>           Read the history file and append its contents to the\n                        history list.\n    -w <file>           Write out the current history list to the history\n                        file.\n    -h, --help          Display help\n")), None));
        assert_eq!(input.len(), 0);
    }

    #[test]
    fn test_bash_parser_return() {
        let (core, _): (ShellCore, UserStream) = ShellCore::new(None, 32, Box::new(Bash::new()));
        let parser: Bash = Bash::new();
        //Simple case
        let mut input: VecDeque<String> = parser.readline(&String::from("0")).unwrap();
        assert_eq!(parser.parse_return(&mut input).unwrap(), ShellStatement::Return(0));
        assert_eq!(input.len(), 0); //Should be empty
        //Simple case
        let mut input: VecDeque<String> = parser.readline(&String::from("128")).unwrap();
        assert_eq!(parser.parse_return(&mut input).unwrap(), ShellStatement::Return(128));
        assert_eq!(input.len(), 0); //Should be empty
        //Bad case
        let mut input: VecDeque<String> = parser.readline(&String::from("foobar")).unwrap();
        assert_eq!(parser.parse_return(&mut input).unwrap(), ShellStatement::Return(2));
        assert_eq!(input.len(), 0); //Should be empty
        //No arg
        let mut input: VecDeque<String> = VecDeque::new();
        assert_eq!(parser.parse_return(&mut input).unwrap(), ShellStatement::Return(0));
        assert_eq!(input.len(), 0); //Should be empty
    }

    //@! States

    #[test]
    fn test_bash_parser_state_is_on_top() {
        let mut parser_state: BashParserState = BashParserState::new();
        //Stack
        parser_state.stack_state(BashParserBlock::Quoted('"'));
        assert!(parser_state.is_on_top(BashParserBlock::Quoted('"')));
        //Check another quote, but with tick
        assert!(! parser_state.is_on_top(BashParserBlock::Quoted('\'')));
        assert!(! parser_state.is_on_top(BashParserBlock::Escaped));
        //Pop
        parser_state.pop();
        assert!(! parser_state.is_on_top(BashParserBlock::Quoted('"')));
    }

    #[test]
    fn test_bash_parser_state_funcs() {
        let mut states: BashParserState = BashParserState::new();
        assert!(! states.is_in_for_loop());
        states.open_for_loop();
        assert!(states.is_on_top(BashParserBlock::ForLoop));
        assert!(states.is_in_for_loop());
        //Open while loop on top
        states.open_while_loop();
        assert!(! states.is_on_top(BashParserBlock::ForLoop));
        assert!(states.is_on_top(BashParserBlock::WhileLoop));
        assert!(states.is_in_while_loop());
        assert!(! states.is_in_for_loop()); //For loop no more on top
        assert!(states.close_for_loop().is_err()); //Can't close for loop before while
        assert!(states.close_while_loop().is_ok());
        assert!(! states.is_on_top(BashParserBlock::WhileLoop));
        //Now for loop can be closed
        assert!(states.close_for_loop().is_ok());
    }

    #[test]
    fn test_bash_parser_state_update() {
        let mut states: BashParserState = BashParserState::new();
        //Check escape
        assert!(states.update_state('\\').is_none());
        assert!(states.is_on_top(BashParserBlock::Escaped));
        assert!(states.is_escaped());
        assert_eq!(states.previous_char, '\\');
        //Try with another backslash
        states.update_state('\\');
        assert!(! states.is_on_top(BashParserBlock::Escaped));
        assert_eq!(states.previous_char, '\\');
        //Check if escape terminates
        assert!(states.update_state('\\').is_none());
        assert!(states.is_on_top(BashParserBlock::Escaped));
        assert!(states.is_escaped());
        assert_eq!(states.previous_char, '\\');
        assert!(states.update_state('a').is_none());
        assert!(! states.is_on_top(BashParserBlock::Escaped));
        assert!(! states.is_escaped());
        //Try quotes
        states.update_state('"');
        assert!(states.is_on_top(BashParserBlock::Quoted('"')));
        assert!(states.is_quoted());
        assert_eq!(states.previous_char, '"');
        //Try a '
        states.update_state('\'');
        assert!(states.is_on_top(BashParserBlock::Quoted('"')));
        //Close quotes
        states.update_state('"');
        assert!(! states.is_on_top(BashParserBlock::Quoted('"')));

        //Try quotes with tick
        states.update_state('\'');
        assert!(states.is_on_top(BashParserBlock::Quoted('\'')));
        assert!(states.is_quoted());
        assert_eq!(states.previous_char, '\'');
        //Try a "
        states.update_state('"');
        assert!(states.is_on_top(BashParserBlock::Quoted('\'')));
        //Close quotes
        states.update_state('\'');
        assert!(! states.is_on_top(BashParserBlock::Quoted('\'')));

        //Try expressions
        states.update_state('$'); //Prepare expression with $
        states.update_state('('); //Open expression now
        assert!(states.is_on_top(BashParserBlock::Expression('(')));
        assert!(states.is_inside_expression());
        //Close expression
        states.update_state(')');
        assert!(! states.is_on_top(BashParserBlock::Expression('(')));
        assert!(! states.is_inside_expression());
        //@! Bad expressions
        //Try to close an unopened expression 
        assert_eq!(states.update_state(')').unwrap(), ParserErrorCode::BadToken);
        //Try to open an expression while quoted
        states.update_state('"');
        assert!(states.is_quoted());
        //Open expression (won't open)
        states.update_state('(');
        assert_eq!(states.previous_char, '(');
        assert!(! states.is_inside_expression());
        states.update_state('"');
        assert!(! states.is_quoted());
        //Try to close an expression while quoted
        states.update_state('`');
        assert!(states.is_inside_expression());
        states.update_state('"');
        assert!(states.is_quoted());
        states.update_state('`');
        assert!(! states.is_inside_expression()); //Not on top
        //Close quote and expression
        states.update_state('"');
        //Should still be in expression
        assert!(states.is_inside_expression());
        assert!(! states.is_quoted());
        states.update_state('`');
        assert!(! states.is_inside_expression());
    }

}
