use crate::{config::ConfFile, err::StatixErr};

pub fn run() -> Result<(), StatixErr> {
    println!("{}", ConfFile::dump(&ConfFile::default()));
    Ok(())
}
