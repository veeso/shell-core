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

use crate::{ParseStatement, ParserError, ParserErrorCode, ShellCore, ShellExpression, ShellStatement, TaskRelation};
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

//TODO: remember to resolve path before using them

impl ParseStatement for Bash {
    fn parse(&self, core: &ShellCore, statement: &String) -> Result<ShellExpression, ParserError> {
        //Instantiate BashParserState
        let mut state: BashParserState = BashParserState::new();
        let mut argv: VecDeque<String> = match self.readline(statement) {
            Ok(argv) => argv,
            Err(err) => return Err(err)
        };
        self.parse_lines(core, state, argv)
    }
}

impl Bash {

    /// ### new
    /// 
    /// Instantiates a new Bash
    pub fn new() -> Bash {
        Bash {}
    }

    /// ### parse_lines
    /// 
    /// Recursive function which parse lines
    fn parse_lines(&self, core: &ShellCore, mut state: BashParserState, mut input: VecDeque<String>) -> Result<ShellExpression, ParserError> {
        //Start iterating
        let mut statements: Vec<(ShellStatement, TaskRelation)> = Vec::new();
        loop {

        }
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
        for line in input.split("\n") { //Iter over lines
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
            }
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

    /// ### parse_cd
    /// 
    /// Parse CD statement. Returns the ShellStatement parsed.
    /// Cd is already removed from input
    fn parse_cd(&self, core: &ShellCore, state: &mut BashParserState, argv: &mut VecDeque<String>) -> Result<ShellStatement, ParserError> {
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
                    let first_arg: String = argv.front().unwrap().clone();
                    PathBuf::from(first_arg.as_str().trim())
                }
            }
        };
        //Remove useless arguments
        self.cut_argv_to_delim(argv);
        //Return Cd statement
        Ok(ShellStatement::Cd(dir))
    }

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
        assert_eq!(parser.readline(&String::from("cd /tmp/\ncd /home/")).unwrap(), vec![String::from("cd"), String::from("/tmp/"), String::from("cd"), String::from("/home/")]);
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
    fn test_bash_parser_cd() {
        let (core, _): (ShellCore, UserStream) = ShellCore::new(None, 32, Box::new(Bash::new()));
        let mut states: BashParserState = BashParserState::new();
        let parser: Bash = Bash::new();
        //Parse some CD statements
        //Simple case
        let mut input: VecDeque<String> = parser.readline(&String::from("/tmp")).unwrap();
        assert_eq!(parser.parse_cd(&core, &mut states, &mut input).unwrap(), ShellStatement::Cd(PathBuf::from("/tmp")));
        assert_eq!(input.len(), 0); //Should be empty
        //With semicolon
        let mut input: VecDeque<String> = parser.readline(&String::from("/tmp;")).unwrap();
        assert_eq!(parser.parse_cd(&core, &mut states, &mut input).unwrap(), ShellStatement::Cd(PathBuf::from("/tmp")));
        assert_eq!(input, vec![String::from(";")]); //Should be empty
        //With newline
        let mut input: VecDeque<String> = parser.readline(&String::from("/tmp\n")).unwrap();
        assert_eq!(parser.parse_cd(&core, &mut states, &mut input).unwrap(), ShellStatement::Cd(PathBuf::from("/tmp")));
        assert_eq!(input.len(), 0); //Should be empty
        //Too many arguments
        let mut input: VecDeque<String> = parser.readline(&String::from("/tmp /home/")).unwrap();
        assert!(parser.parse_cd(&core, &mut states, &mut input).is_err());
        assert_eq!(input.len(), 0); //Should be empty
        //Too many arguments 2
        let mut input: VecDeque<String> = parser.readline(&String::from("/tmp /home/;")).unwrap();
        assert!(parser.parse_cd(&core, &mut states, &mut input).is_err());
        assert_eq!(input, vec![String::from(";")]); //Should be empty
        //False too many arguments
        let mut input: VecDeque<String> = parser.readline(&String::from("/tmp ;")).unwrap();
        assert_eq!(parser.parse_cd(&core, &mut states, &mut input).unwrap(), ShellStatement::Cd(PathBuf::from("/tmp")));
        assert_eq!(input, vec![String::from(";")]); //Should be empty
        //Too many arguments due to escape
        let mut input: VecDeque<String> = parser.readline(&String::from("/tmp \\;")).unwrap();
        assert!(parser.parse_cd(&core, &mut states, &mut input).is_err());
        assert_eq!(input.len(), 0); //Should be empty
        //Quotes
        let mut input: VecDeque<String> = parser.readline(&String::from("\"/home\"")).unwrap();
        assert_eq!(parser.parse_cd(&core, &mut states, &mut input).unwrap(), ShellStatement::Cd(PathBuf::from("/home/")));
        assert_eq!(input.len(), 0); //Should be empty
        //Escaped quotes
        let mut input: VecDeque<String> = parser.readline(&String::from("/home/\\\"foo\\\"")).unwrap();
        assert_eq!(parser.parse_cd(&core, &mut states, &mut input).unwrap(), ShellStatement::Cd(PathBuf::from("/home/\"foo\"")));
        assert_eq!(input.len(), 0); //Should be empty
        //With and
        let mut input: VecDeque<String> = parser.readline(&String::from("/tmp &&")).unwrap();
        assert_eq!(parser.parse_cd(&core, &mut states, &mut input).unwrap(), ShellStatement::Cd(PathBuf::from("/tmp")));
        assert_eq!(input, vec![String::from("&&")]); //Should be &&
    }

    #[test]
    fn test_bash_parser_state_new() {
        let parser_state: BashParserState = BashParserState::new();
        assert_eq!(parser_state.states.len(), 0);
        assert!(parser_state.empty());
        assert_eq!(parser_state.previous_char, ' ');
    }

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
