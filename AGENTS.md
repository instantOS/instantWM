This is a Rust rewrite of a C dwm fork. The Rust rewrite aims to reproduce the
functionality of the C version, but to make use of what Rust can do which C
cannot. 
As such, things like the generic Arg struct have been eliminated in favor of
more type safe and comprehensible systems. 

The Rust rewrite is located in ./rust
The c source files are in ./

Agents are not allowed to use git at all. No commits, no diffs, no checkouts. 
When git is needed, ask the user to do it. 
