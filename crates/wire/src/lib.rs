use std::io::{Read, Write};
pub const CAPACITY: usize = 4 * 1024 * 1024;
pub trait Framing {
    fn read_frame(&mut self) -> std::io::Result<Vec<u8>>;
    fn write_frame(&mut self, bytes: &[u8]) -> std::io::Result<()>;
}
impl<T: Read + Write> Framing for T {
    fn read_frame(&mut self) -> std::io::Result<Vec<u8>> {
        let mut head = [0u8; 4];
        self.read_exact(&mut head)?;
        let size = u32::from_le_bytes(head) as usize;
        if size > CAPACITY {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "frame too large",
            ));
        }
        let mut bytes = vec![0; size];
        self.read_exact(&mut bytes)?;
        Ok(bytes)
    }
    fn write_frame(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if bytes.len() > CAPACITY {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "frame too large",
            ));
        }
        self.write_all(&(bytes.len() as u32).to_le_bytes())?;
        self.write_all(bytes)?;
        self.flush()
    }
}
pub trait SocketPathing {
    fn public(&self) -> std::path::PathBuf;
    fn meta(&self) -> std::path::PathBuf;
    fn emission(&self) -> std::path::PathBuf;
    fn public_translator(&self) -> std::path::PathBuf;
    fn meta_translator(&self) -> std::path::PathBuf;
}
#[derive(Clone, Debug)]
pub struct Place {
    pub runtime: std::path::PathBuf,
}
impl SocketPathing for Place {
    fn public(&self) -> std::path::PathBuf {
        self.runtime.join("energy.sock")
    }
    fn meta(&self) -> std::path::PathBuf {
        self.runtime.join("energy-meta.sock")
    }
    fn emission(&self) -> std::path::PathBuf {
        self.runtime.join("energy-emission.sock")
    }
    fn public_translator(&self) -> std::path::PathBuf {
        self.runtime.join("energy-translator.sock")
    }
    fn meta_translator(&self) -> std::path::PathBuf {
        self.runtime.join("energy-meta-translator.sock")
    }
}
