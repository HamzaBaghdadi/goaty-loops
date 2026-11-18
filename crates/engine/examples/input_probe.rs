//! Lists the default audio input device and its format (does not record).
use cpal::traits::{DeviceTrait, HostTrait};
fn main() {
    let host = cpal::default_host();
    match host.default_input_device() {
        Some(d) => println!("input: {} {:?}", d.name().unwrap_or_default(), d.default_input_config()),
        None => println!("no input device"),
    }
}
