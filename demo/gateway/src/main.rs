#![allow(dead_code)]



mod config;
mod error;
mod proxy;
mod tunnel;
mod peer;
mod service;
mod gateway;

#[macro_use]
extern crate log;

fn main() {
    println!("Hello, world!");

    // ConfigLoader::load(json)
}