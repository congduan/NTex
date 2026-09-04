//! PNG 导出（纯 Rust：zlib deflate + PNG 容器，RFC 2083）。
//!
//! 依赖最小化：tiny-skia 首选路径不可用（离线无缓存，见 lib.rs 偏差说明），
//! 故自研编码。位深 8、颜色类型 6（RGBA）、单 IDAT、filter 0（None）——
//! 压缩率非目标，正确性与可移植优先；`crc32`/`adler32` 为 RFC 1952 标准实现。

use crate::raster::Pixmap;
use std::fmt;

/// PNG 编码错误（当前实现不会失败，保留 Result 形态给未来后端）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngError(pub String);

impl fmt::Display for PngError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PNG 编码失败：{}", self.0)
    }
}

impl std::error::Error for PngError {}

type Result<T> = std::result::Result<T, PngError>;

/// 把 RGBA 像素缓冲编码为 PNG 字节。
pub fn encode_png(pm: &Pixmap) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(pm.data().len() / 2 + 1024);
    out.extend([0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    // IHDR：宽、高、位深 8、RGBA(6)、压缩 0、滤波 0、隔行 0。
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend(pm.width().to_be_bytes());
    ihdr.extend(pm.height().to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    push_chunk(&mut out, b"IHDR", &ihdr);

    // 原始数据：每行前加 filter 字节 0。
    let stride = pm.width() as usize * 4;
    let mut raw = Vec::with_capacity((stride + 1) * pm.height() as usize);
    for row in pm.data().chunks(stride) {
        raw.push(0);
        raw.extend(row);
    }
    let idat = zlib_deflate(&raw);
    push_chunk(&mut out, b"IDAT", &idat);
    push_chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

/// 写一个 PNG chunk（长度 + 类型 + 数据 + CRC32）。
fn push_chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let crc_start = out.len();
    out.extend(tag);
    out.extend(data);
    let crc = crc32(&out[crc_start..]);
    out.extend(crc.to_be_bytes());
}

/// zlib 流：0x78 0x01（最快压缩）+ deflate stored 块 + adler32。
fn zlib_deflate(raw: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    // stored 块按 65535 字节切片：BFINAL/BTYPE(00) + LEN + NLEN + 原字节。
    for (i, chunk) in raw.chunks(65_535).enumerate() {
        let last = if i == raw.len().div_ceil(65_535) - 1 {
            1
        } else {
            0
        };
        out.push(last);
        out.extend((chunk.len() as u16).to_le_bytes());
        out.extend((!(chunk.len() as u16)).to_le_bytes());
        out.extend(chunk);
    }
    // 空输入也要有一个终止块（RFC 1951 §3.2.7）。
    if raw.is_empty() {
        out.extend([1, 0, 0, 0xFF, 0xFF]);
    }
    out.extend(adler32(raw).to_be_bytes());
    out
}

/// CRC-32（ISO 3309 / ITU-T V.42，PNG 标准表驱动）。
fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, entry) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *entry = c;
    }
    let mut crc = 0xFFFF_FFFF;
    for &b in data {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

/// Adler-32（RFC 1950 §8.2）。
fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65_521;
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_signature_and_ihdr() {
        let pm = Pixmap::new(3, 2);
        let png = encode_png(&pm).unwrap();
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        // 第一个 chunk：长度 13 + "IHDR"。
        assert_eq!(&png[8..12], &13u32.to_be_bytes());
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[16..20], &3u32.to_be_bytes());
        assert_eq!(&png[20..24], &2u32.to_be_bytes());
    }

    #[test]
    fn zlib_checksums_match_rfc_vectors() {
        assert_eq!(adler32(b"abc"), 0x024d_0127);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}
