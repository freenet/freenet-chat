#![allow(non_snake_case)]

use dioxus::prelude::*;
use dioxus_logger::tracing::{info, Level};

mod components;
mod example_data;
mod util;
mod global_context;
use components::app::App;

use common::state::member::{MemberId, Member};
use freenet_scaffold::util::FastHash;

fn main() {
    // Init logger
    dioxus_logger::init(Level::DEBUG).expect("failed to init logger");
    info!("starting app");
    launch(App);
}
