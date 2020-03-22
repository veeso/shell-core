//! # Parsers
//!
//! `parsers` is contains the shell parsers provided by Shell-Core crate

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

pub mod bash;

use crate::ParserError;
use crate::ParserErrorCode;

impl ParserError {
    /// ## new
    ///
    /// Instantiate a new Task Error struct
    pub(crate) fn new(code: ParserErrorCode, message: String) -> ParserError {
        ParserError {
            code: code,
            message: message,
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_parser_error_new() {
        let error: ParserError = ParserError::new(ParserErrorCode::BadToken, String::from("Bad token at row 0, column 12"));
        assert_eq!(error.code, ParserErrorCode::BadToken);
        assert_eq!(error.message, String::from("Bad token at row 0, column 12"));
    }
}
