//! # Streams
//!
//! `streams` provides the implementations for ShellStream and UserStream

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

use crate::{ShellStream, ShellStreamMessage, UserStream, UserStreamMessage};

use std::sync::mpsc;

/// ## new_streams
/// 
/// Generate a new pair of streams
pub(crate) fn new_streams() -> (ShellStream, UserStream) {
    //Generate channels
    let (sstream_sender, sstream_receiver) = mpsc::channel();
    let (ustream_sender, ustream_receiver) = mpsc::channel();
    let sstream = ShellStream::new(ustream_receiver, sstream_sender);
    let ustream = UserStream::new(sstream_receiver, ustream_sender);
    (sstream, ustream)
}

impl ShellStream {

    /// ### new
    /// 
    /// Instantiate a new ShellStream
    pub(crate) fn new(receiver: mpsc::Receiver<UserStreamMessage>, sender: mpsc::Sender<ShellStreamMessage>) -> ShellStream {
        ShellStream {
            receiver: receiver,
            sender: sender
        }
    }

    /// ### receive
    /// 
    /// Receive messages from Shell Stream. Errors doesn't include empty inbox, in that case an empty vector is returned
    pub fn receive(&self) -> Result<Vec<UserStreamMessage>, ()> {
        let mut inbox: Vec<UserStreamMessage> = Vec::new();
        loop {
            match self.receiver.try_recv() {
                Ok(message) => inbox.push(message),
                Err(err) => match err {
                    mpsc::TryRecvError::Empty => break,
                    _ => return Err(())
                }
            }
        }
        Ok(inbox)
    }

    /// ### send
    /// 
    /// Send a message to the UserStream receiver
    pub fn send(&self, message: ShellStreamMessage) -> bool {
        match self.sender.send(message) {
            Ok(()) => true,
            Err(_) => false
        }
    }

}

impl UserStream {

    /// ### new
    /// 
    /// Instantiate a new UserStream
    pub(crate) fn new(receiver: mpsc::Receiver<ShellStreamMessage>, sender: mpsc::Sender<UserStreamMessage>) -> UserStream {
        UserStream {
            receiver: receiver,
            sender: sender
        }
    }

    /// ### receive
    /// 
    /// Receive messages from Shell Stream. Errors doesn't include empty inbox, in that case an empty vector is returned
    pub fn receive(&self) -> Result<Vec<ShellStreamMessage>, ()> {
        let mut inbox: Vec<ShellStreamMessage> = Vec::new();
        loop {
            match self.receiver.try_recv() {
                Ok(message) => inbox.push(message),
                Err(err) => match err {
                    mpsc::TryRecvError::Empty => break,
                    _ => return Err(())
                }
            }
        }
        Ok(inbox)
    }

    /// ### send
    /// 
    /// Send a message to the UserStream receiver
    pub fn send(&self, message: UserStreamMessage) -> bool {
        match self.sender.send(message) {
            Ok(()) => true,
            Err(_) => false
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    use std::mem::drop;

    #[test]
    fn test_streams_success() {
        let (sstream, ustream): (ShellStream, UserStream) = new_streams();
        //Try to communicate (Send output)
        assert!(sstream.send(ShellStreamMessage::Output((Some(String::from("Hi there!")), None))));
        //Try to receive
        let inbox: Vec<ShellStreamMessage> = ustream.receive().unwrap();
        //Len must be 1
        assert_eq!(inbox.len(), 1);
        //Verify message is Output
        match &inbox[0] {
            ShellStreamMessage::Output((stdout, stderr)) => {
                assert_eq!(*stdout.as_ref().unwrap(), String::from("Hi there!"));
                assert!(stderr.is_none());
            },
            _ => panic!("Unexpected stream message")
        }
        //@!Try to communicate from ustream to sstream
        assert!(ustream.send(UserStreamMessage::Input(String::from("Hello!"))));
        //Try to receive
        let inbox: Vec<UserStreamMessage> = sstream.receive().unwrap();
        //Len must be 1
        assert_eq!(inbox.len(), 1);
        //Verify message is Output
        match &inbox[0] {
            UserStreamMessage::Input(stdin) => {
                assert_eq!(*stdin, String::from("Hello!"));
            },
            _ => panic!("Unexpected stream message")
        }
    }

    #[test]
    fn test_streams_empty_inbox() {
        let (sstream, ustream): (ShellStream, UserStream) = new_streams();
        assert_eq!(sstream.receive().unwrap().len(), 0);
        assert_eq!(ustream.receive().unwrap().len(), 0);
    }

    #[test]
    fn test_streams_sstream_hangup() {
        let (sstream, ustream): (ShellStream, UserStream) = new_streams();
        drop(sstream);
        //Should return false
        assert!(!ustream.send(UserStreamMessage::Kill));
        //Should return error
        assert!(ustream.receive().is_err());
    }

    #[test]
    fn test_streams_ustream_hangup() {
        let (sstream, ustream): (ShellStream, UserStream) = new_streams();
        drop(ustream);
        //Should return false
        assert!(!sstream.send(ShellStreamMessage::Output((None, None))));
        //Should return error
        assert!(sstream.receive().is_err());
    }

}
