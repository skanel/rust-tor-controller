extern crate log;
extern crate env_logger;
extern crate tor_controller;

use tor_controller::utils;

fn main() {
    env_logger::init();

    println!("Tor Version = {:?}", utils::get_system_tor_version(None));
}
