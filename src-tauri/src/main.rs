#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    cswitch::run().expect("failed to run CSwitch");
}
