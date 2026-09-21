//! PNG 位图 → PDF Image XObject 素材（图片管线 Step B）。
//!
//! 输入：PNG 文件字节（`\pdfximage` 登记的图源，DVI xxx 载荷只带文件名，
//! PDF 侧按 `--input-path` 找回）。输出：PDF 侧要的三件套——像素宽高、
//! FlateDecode 压缩的 DeviceRGB 主流、（若源含 alpha）FlateDecode 压缩的
//! DeviceGray 软掩码流。
//!
//! 支持面按 Transformer 论文图源实态裁剪：8-bit、非隔行、颜色类型 2
//! (RGB) / 6 (RGBA)。alpha 不丢弃也不原样嵌入（PDF 无 4 分量设备色空间，
//! IDAT 直拷路线对 RGBA 走不通）：inflate → unfilter 后把 alpha 合成到
//! **白底**（图注/正文都是白纸，合成结果即印刷外观），输出纯 RGB 流。
//!
//! zlib 压缩用 `flate2`（Cargo.lock 已有，经 miniz_oxide 纯 Rust 后端）。

use std::io::{Read as _, Write as _};

/// 解码结果：PDF Image XObject 所需素材。
pub struct DecodedPng {
    pub width: u32,
    pub height: u32,
    /// DeviceRGB 样本流（FlateDecode 后的原始行序：每像素 3 字节，行间无滤
    /// 波字节——预测器参数不用，滤在解码期已完成）。
    pub rgb: Vec<u8>,
    /// DeviceGray 软掩码样本流（每像素 1 字节）。仅源含 alpha 时提供；
    /// None = 完全不透明（无需 /SMask）。
    pub smask: Option<Vec<u8>>,
}

/// PNG 解码失败（含"特性不支持"——调用方降级为跳图 + 警告，不 panic）。
#[derive(Debug)]
pub struct PngError(pub String);

impl std::fmt::Display for PngError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PNG 解码失败：{}", self.0)
    }
}

impl std::error::Error for PngError {}

/// 解码 PNG 字节为 PDF 素材。不支持的颜色类型/位深/隔行 → Err（优雅降级）。
pub fn decode_png(data: &[u8]) -> Result<DecodedPng, PngError> {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if data.len() < 8 || data[..8] != SIG {
        return Err(PngError("非 PNG 签名".into()));
    }

    // 逐 chunk：收集 IHDR 头域、拼接 IDAT、忽略 ancillary。
    let mut pos = 8usize;
    let mut ihdr: Option<[u8; 13]> = None;
    let mut idat: Vec<u8> = Vec::new();
    while pos + 8 <= data.len() {
        let len = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
            as usize;
        let Some(typ) = data.get(pos + 4..pos + 8) else {
            break;
        };
        let Some(body) = data.get(pos + 8..pos + 8 + len) else {
            return Err(PngError("chunk 截断".into()));
        };
        match typ {
            b"IHDR" => {
                let mut h = [0u8; 13];
                h.copy_from_slice(body);
                ihdr = Some(h);
            }
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {} // PLTE/gAMA/…：RGB/RGBA 路径不需要
        }
        pos += 12 + len; // len + type + data + crc
    }
    let Some([w0, w1, w2, w3, h0, h1, h2, h3, depth, ctype, comp, filter, interlace]) = ihdr
    else {
        return Err(PngError("缺 IHDR".into()));
    };
    let width = u32::from_be_bytes([w0, w1, w2, w3]);
    let height = u32::from_be_bytes([h0, h1, h2, h3]);
    if width == 0 || height == 0 || width > 1 << 20 || height > 1 << 20 {
        return Err(PngError(format!("尺寸异常 {width}x{height}")));
    }
    if depth != 8 || comp != 0 || filter != 0 || interlace != 0 {
        return Err(PngError(format!(
            "不支持的 PNG 形态：位深 {depth} 压缩 {comp} 滤波 {filter} 隔行 {interlace}（仅 8-bit 非隔行）"
        )));
    }
    let channels = match ctype {
        2 => 3usize, // RGB
        6 => 4usize, // RGBA
        other => {
            return Err(PngError(format!(
                "不支持的 PNG 颜色类型 {other}（仅 RGB 2 / RGBA 6；灰度/调色板可按需扩展）"
            )))
        }
    };

    // inflate → 逐行 unfilter（RFC 2083 §6；滤波字节还原**作用于字节**，
    // 与通道数无关，唯 bpp 以字节计）。
    let raw = inflate(&idat)?;
    let stride = width as usize * channels;
    let mut prev = vec![0u8; stride];
    let mut cur = vec![0u8; stride];
    let mut px = vec![0u8; stride * height as usize];
    let mut src = 0usize;
    for row in 0..height as usize {
        let Some(&ftag) = raw.get(src) else {
            return Err(PngError("像素流截断".into()));
        };
        src += 1;
        let Some(line) = raw.get(src..src + stride) else {
            return Err(PngError("像素流截断（行不完整）".into()));
        };
        src += stride;
        for (i, &b) in line.iter().enumerate() {
            let a = if i >= channels { cur[i - channels] } else { 0 };
            let b_up = prev[i];
            let c = if i >= channels { prev[i - channels] } else { 0 };
            cur[i] = match ftag {
                0 => b,                                // None
                1 => b.wrapping_add(a),                // Sub
                2 => b.wrapping_add(b_up),             // Up
                3 => b.wrapping_add(((a as u16 + b_up as u16) / 2) as u8), // Average
                4 => b.wrapping_add(paeth(a, b_up, c)), // Paeth
                other => return Err(PngError(format!("未知滤波类型 {other}"))),
            };
        }
        px[row * stride..(row + 1) * stride].copy_from_slice(&cur);
        std::mem::swap(&mut prev, &mut cur);
    }

    // 拆通道：RGB 主流 +（RGBA）白底合成。白底合成只改 RGB（alpha 已消耗），
    // 软掩码流另存原 alpha——下游要真透明时（未来 /SMask）直接可用。
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    let mut smask = if channels == 4 {
        Some(Vec::with_capacity(width as usize * height as usize))
    } else {
        None
    };
    for p in px.chunks_exact(channels) {
        let (r, g, b) = (p[0], p[1], p[2]);
        if channels == 4 {
            let a = p[3];
            // 合成到白底：out = c*a + 255*(1-a)（整数近似：+127 圆整）。
            let over = |c: u8| {
                (c as u32 * a as u32 + 255 * (255 - a as u32) + 127) / 255
            } as u8;
            rgb.extend_from_slice(&[over(r), over(g), over(b)]);
            smask.as_mut().unwrap().push(a);
        } else {
            rgb.extend_from_slice(&[r, g, b]);
        }
    }

    Ok(DecodedPng {
        width,
        height,
        rgb: deflate(&rgb),
        smask: smask.map(|s| deflate(&s)),
    })
}

/// Paeth 预测器（RFC 2083 §6.1）。
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i32 + b as i32 - c as i32;
    let (pa, pb, pc) = (p - a as i32, p - b as i32, p - c as i32);
    let (pa, pb, pc) = (pa.abs(), pb.abs(), pc.abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// zlib inflate（PNG IDAT = zlib 流；flate2 读端）。
fn inflate(data: &[u8]) -> Result<Vec<u8>, PngError> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|e| PngError(format!("zlib 解压：{e}")))?;
    Ok(out)
}

/// zlib deflate（PDF FlateDecode 同为 zlib 流；压缩级默认 6，图源是线图时
/// 相对原始 RGB 仍大副收缩）。
fn deflate(data: &[u8]) -> Vec<u8> {
    let mut enc =
        flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = enc.write_all(data);
    enc.finish().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 手工构造一张 1x2 RGBA PNG（filter 0 直存），走解码全链。
    fn tiny_rgba_png() -> Vec<u8> {
        let raw = [
            0u8, 255, 0, 0, 255, // 行0 filter0：红不透明
            0, 0, 0, 255, 128, // 行1 filter0：蓝半透明
        ];
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let mut ihdr = Vec::new();
        ihdr.extend(1u32.to_be_bytes());
        ihdr.extend(2u32.to_be_bytes());
        ihdr.extend([8, 6, 0, 0, 0]);
        push(&mut out, b"IHDR", &ihdr);
        push(&mut out, b"IDAT", &deflate(&raw));
        push(&mut out, b"IEND", &[]);
        out
    }

    fn push(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(tag);
        out.extend_from_slice(data);
        let mut c = Crc32::new();
        c.update(&out[start..]);
        out.extend(c.sum().to_be_bytes());
    }

    /// CRC32（RFC 1952；测试自足，不引 ntex-backend）。
    struct Crc32(u32);
    impl Crc32 {
        fn new() -> Self {
            Self(0xFFFF_FFFF)
        }
        fn update(&mut self, data: &[u8]) {
            for &b in data {
                self.0 ^= b as u32;
                for _ in 0..8 {
                    self.0 = if self.0 & 1 != 0 {
                        (self.0 >> 1) ^ 0xEDB8_8320
                    } else {
                        self.0 >> 1
                    };
                }
            }
        }
        fn sum(&self) -> u32 {
            self.0 ^ 0xFFFF_FFFF
        }
    }

    #[test]
    fn decodes_rgba_and_composites_over_white() {
        let d = decode_png(&tiny_rgba_png()).expect("解码成功");
        assert_eq!((d.width, d.height), (1, 2));
        let rgb = inflate(&d.rgb).expect("rgb 流合法 zlib");
        // 红 a=255 原样；蓝 a=128 → 0*0.502+255*0.498 ≈ 127
        assert_eq!(&rgb[..], &[255, 0, 0, 127, 127, 255]);
        let a = inflate(d.smask.as_ref().expect("含掩码")).expect("a 流合法 zlib");
        assert_eq!(&a[..], &[255, 128]);
    }

    #[test]
    fn rejects_unsupported_forms() {
        // 签名对但无 IHDR
        let mut bad = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        push(&mut bad, b"IEND", &[]);
        assert!(decode_png(&bad).is_err());
        assert!(decode_png(b"not a png").is_err());
    }
}
