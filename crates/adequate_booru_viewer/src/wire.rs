use anyhow::{Context as _, Result, bail};

pub struct Sink {
    bytes: Vec<u8>,
}

impl Sink {
    pub fn with_magic(magic: &[u8]) -> Self {
        let mut bytes = Vec::with_capacity(256);
        bytes.extend_from_slice(magic);
        Self { bytes }
    }

    pub fn bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn var(&mut self, mut value: u64) {
        while value >= 0x80 {
            self.u8((value as u8) | 0x80);
            value >>= 7;
        }
        self.u8(value as u8);
    }

    pub fn bytes_raw(&mut self, bytes: &[u8]) {
        self.var(bytes.len() as u64);
        self.bytes.extend_from_slice(bytes);
    }

    pub fn str(&mut self, value: &str) {
        self.bytes_raw(value.as_bytes());
    }

    pub fn opt_str(&mut self, value: Option<&str>) {
        match value {
            Some(value) => {
                self.u8(1);
                self.str(value);
            }
            None => self.u8(0),
        }
    }
}

pub struct Blade<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Blade<'a> {
    pub fn new(bytes: &'a [u8], magic: &[u8]) -> Result<Self> {
        let Some(rest) = bytes.strip_prefix(magic) else {
            bail!("wire record magic mismatch");
        };
        Ok(Self { bytes: rest, at: 0 })
    }

    pub fn done(&self) -> Result<()> {
        if self.at == self.bytes.len() {
            Ok(())
        } else {
            bail!(
                "wire record has {} trailing bytes",
                self.bytes.len().saturating_sub(self.at)
            )
        }
    }

    pub fn is_done(&self) -> bool {
        self.at == self.bytes.len()
    }

    pub fn u8(&mut self) -> Result<u8> {
        let value = *self
            .bytes
            .get(self.at)
            .context("wire u8 exceeds record boundary")?;
        self.at += 1;
        Ok(value)
    }

    pub fn u32(&mut self) -> Result<u32> {
        let bytes = self.fixed::<4>()?;
        Ok(u32::from_le_bytes(bytes))
    }

    pub fn i32(&mut self) -> Result<i32> {
        let bytes = self.fixed::<4>()?;
        Ok(i32::from_le_bytes(bytes))
    }

    pub fn var(&mut self) -> Result<u64> {
        let mut shift = 0_u32;
        let mut out = 0_u64;
        loop {
            let byte = self.u8()?;
            out |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(out);
            }
            shift += 7;
            if shift >= 64 {
                bail!("wire varint exceeds u64");
            }
        }
    }

    pub fn bytes_raw(&mut self) -> Result<&'a [u8]> {
        let len = usize::try_from(self.var()?).context("wire byte length exceeds usize")?;
        let end = self
            .at
            .checked_add(len)
            .context("wire byte length overflows cursor")?;
        if end > self.bytes.len() {
            bail!("wire byte blob exceeds record boundary");
        }
        let bytes = &self.bytes[self.at..end];
        self.at = end;
        Ok(bytes)
    }

    pub fn string(&mut self) -> Result<String> {
        let bytes = self.bytes_raw()?;
        std::str::from_utf8(bytes)
            .context("wire string is not UTF-8")
            .map(ToOwned::to_owned)
    }

    pub fn opt_string(&mut self) -> Result<Option<String>> {
        match self.u8()? {
            0 => Ok(None),
            1 => self.string().map(Some),
            tag => bail!("invalid optional string tag {tag}"),
        }
    }

    fn fixed<const N: usize>(&mut self) -> Result<[u8; N]> {
        let end = self
            .at
            .checked_add(N)
            .context("wire fixed read overflows cursor")?;
        if end > self.bytes.len() {
            bail!("wire fixed read exceeds record boundary");
        }
        let bytes = self.bytes[self.at..end]
            .try_into()
            .context("wire fixed slice width mismatch")?;
        self.at = end;
        Ok(bytes)
    }
}
