use std::collections::HashMap;

use crate::debugger_command::DebuggerCommand;
use crate::dwarf_data::{DwarfData, Error as DwarfError};
use crate::inferior::{Inferior, Status};
use nix::sys::signal::Signal::SIGTRAP;
use nix::sys::wait::{WaitPidFlag, WaitStatus};
use rustyline::error::ReadlineError;
use rustyline::Editor;

pub struct Debugger {
    target: String,
    history_path: String,
    readline: Editor<()>,
    inferior: Option<Inferior>,
    debug_data: DwarfData,
    breakpoints: HashMap<usize, u8>,
}
fn parse_address(addr: &str) -> Option<usize> {
    let addr_without_0x = if addr.to_lowercase().starts_with("0x") {
        &addr[2..]
    } else {
        &addr
    };
    usize::from_str_radix(addr_without_0x, 16).ok()
}
impl Debugger {
    /// Initializes the debugger.
    pub fn new(target: &str) -> Debugger {
        let debug_data = match DwarfData::from_file(target) {
            Ok(val) => val,
            Err(DwarfError::ErrorOpeningFile) => {
                println!("Could not open file {}", target);
                std::process::exit(-1);
            }
            Err(DwarfError::DwarfFormatError(err)) => {
                println!("Could not debugging symbols from {}: {:?}", target, err);
                std::process::exit(-1);
            }
        };
        let history_path = format!("{}/.deet_history", std::env::var("HOME").unwrap());
        let mut readline = Editor::<()>::new();
        // Attempt to load history from ~/.deet_history if it exists
        let _ = readline.load_history(&history_path);

        debug_data.print();
        Debugger {
            target: target.to_string(),
            history_path,
            readline,
            inferior: None,
            debug_data,
            breakpoints: HashMap::new(),
        }
    }

    pub fn run(&mut self) {
        loop {
            match self.get_next_command() {
                DebuggerCommand::Run(args) => {
                    if self.inferior.is_some() {
                        self.inferior.as_mut().unwrap().kill();
                        self.inferior = None;
                    }
                    if let Some(inferior) =
                        Inferior::new(&self.target, &args, &mut self.breakpoints)
                    {
                        // Create the inferior
                        self.inferior = Some(inferior);
                        // You may use self.inferior.as_mut().unwrap() to get a mutable reference
                        // to the Inferior object
                        match self
                            .inferior
                            .as_mut()
                            .unwrap()
                            .continue_run(&self.breakpoints)
                            .unwrap()
                        {
                            Status::Exited(exit_code) => {
                                println!("Child exited (status {})", exit_code);
                                self.inferior = None;
                            }
                            Status::Signaled(_sig) => println!("signal"),
                            Status::Stopped(sig, rip) => {
                                println!("Child stopped (signal {})", sig);
                                let line = DwarfData::get_line_from_addr(&self.debug_data, rip);
                                if line.is_some() {
                                    println!("Stopped at {}", line.unwrap());
                                }
                            }
                        }
                    } else {
                        println!("Error starting subprocess");
                    }
                }
                DebuggerCommand::Breakpoint(addrs) => {
                    for addr in addrs {
                        let mut target_addr: usize = 0;
                        if addr.starts_with("*") {
                            if let Some(taddr) = parse_address(addr[1..].to_string().as_str()) {
                                target_addr = taddr;
                            } else {
                                println!("Invalid address {}", addr);
                            }
                        } else if let Some(line) = usize::from_str_radix(addr.as_str(), 10).ok() {
                            if let Some(laddr) = self.debug_data.get_addr_for_line(None, line) {
                                target_addr = laddr;
                            } else {
                                println!("Invalid line number");
                                continue;
                            }
                        } else if let Some(faddr) =
                            self.debug_data.get_addr_for_function(None, addr.as_str())
                        {
                            target_addr = faddr;
                        } else {
                            println!("Usage: b|break|breakpoint *address|line|func");
                            continue;
                        }

                        if !self.breakpoints.contains_key(&target_addr) {
                            println!(
                                "Set breakpoint {} at {:#x}",
                                self.breakpoints.len(),
                                target_addr
                            );
                        }

                        if self.inferior.is_none() {
                            self.breakpoints.insert(target_addr, 0);
                        } else {
                            if let Some(orig_byte) = self
                                .inferior
                                .as_mut()
                                .unwrap()
                                .write_byte(target_addr, 0xcc)
                                .ok()
                            {
                                self.breakpoints.insert(target_addr, orig_byte);
                            } else {
                                println!("Invalid breakpoint address {:#x}", target_addr);
                            }
                        }
                    }
                }
                DebuggerCommand::Backtrace => {
                    if self.inferior.is_none() {
                        println!("Child not running");
                    } else {
                        self.inferior
                            .as_mut()
                            .unwrap()
                            .print_backtrace(&self.debug_data)
                            .unwrap();
                    }
                }
                DebuggerCommand::Continue => {
                    if self.inferior.is_none() {
                        println!("Child not running");
                    } else {
                        match self
                            .inferior
                            .as_mut()
                            .unwrap()
                            .continue_run(&self.breakpoints)
                            .unwrap()
                        {
                            Status::Exited(exit_code) => {
                                println!("Child exited (status {})", exit_code);
                                self.inferior = None;
                            }
                            Status::Signaled(_sig) => println!("signal"),
                            Status::Stopped(sig, rip) => {
                                println!("Child stopped (signal {})", sig);
                                let line = DwarfData::get_line_from_addr(&self.debug_data, rip);
                                if line.is_some() {
                                    println!("Stopped at {}", line.unwrap());
                                }
                            }
                        }
                    }
                }
                DebuggerCommand::Quit => {
                    if self.inferior.is_some() {
                        self.inferior.as_mut().unwrap().kill();
                        self.inferior = None;
                    }
                    return;
                }
            }
        }
    }

    /// This function prompts the user to enter a command, and continues re-prompting until the user
    /// enters a valid command. It uses DebuggerCommand::from_tokens to do the command parsing.
    ///
    /// You don't need to read, understand, or modify this function.
    fn get_next_command(&mut self) -> DebuggerCommand {
        loop {
            // Print prompt and get next line of user input
            match self.readline.readline("(deet) ") {
                Err(ReadlineError::Interrupted) => {
                    // User pressed ctrl+c. We're going to ignore it
                    println!("Type \"quit\" to exit");
                }
                Err(ReadlineError::Eof) => {
                    // User pressed ctrl+d, which is the equivalent of "quit" for our purposes
                    return DebuggerCommand::Quit;
                }
                Err(err) => {
                    panic!("Unexpected I/O error: {:?}", err);
                }
                Ok(line) => {
                    if line.trim().len() == 0 {
                        continue;
                    }
                    self.readline.add_history_entry(line.as_str());
                    if let Err(err) = self.readline.save_history(&self.history_path) {
                        println!(
                            "Warning: failed to save history file at {}: {}",
                            self.history_path, err
                        );
                    }
                    let tokens: Vec<&str> = line.split_whitespace().collect();
                    if let Some(cmd) = DebuggerCommand::from_tokens(&tokens) {
                        return cmd;
                    } else {
                        println!("Unrecognized command.");
                    }
                }
            }
        }
    }
}
