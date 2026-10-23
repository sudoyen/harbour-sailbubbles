use std::fs::File;
use std::io::{self, Read};

pub fn bytes<const N: usize>() -> io::Result<[u8; N]> {
    let mut buf = [0u8; N];
    File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_differ() {
        assert_ne!(bytes::<32>().unwrap(), bytes::<32>().unwrap());
    }
}
