use std::{
    path::{Path, PathBuf},
    sync::Once,
};

use rand::Rng;

use inel_reactor::util;

static TRACING: Once = Once::new();
pub fn setup_tracing() {
    util::set_limits().unwrap();

    TRACING.call_once(|| {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::filter::EnvFilter::from_default_env())
            .init();
    });
}

pub fn temp_file() -> PathBuf {
    let seed = rand::rng()
        .sample_iter(rand::distr::Alphanumeric)
        .take(32)
        .map(|b| b as char)
        .collect::<String>();

    Path::new("/tmp").join(format!("inel-test-{}", seed))
}
