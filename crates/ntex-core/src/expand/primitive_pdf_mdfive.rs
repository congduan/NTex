// `\pdfmdfivesum`：pdfTeX 可展开摘要原语（minted/pdftexcmds 闭环，2026-10-25）。
//
// GT 语义（pdflatex 健康通道实测，gt9/gta 探针）：
//   `\pdfmdfivesum{<text>}` → 32 字符**大写** hex MD5（`abc` →
//   900150983CD24FB0D6963F7D28E17F72，RFC 1321 标准向量逐位核对）；
//   实参先按 `\edef` 语义全展开再取字节（`\pdfmdfivesum{Hello \zz}` 与
//   字面展开结果同摘要）；`file <name>` 前缀变体取文件字节摘要
//   （minted `minted@filemdfivehash` 缓存键用 file 形态）；文件不存在 → 空展开。
//
// RFC 1321 参考实现（Rivet 编译的公共域 C 版）的 Rust 直译：
// 确定性、零外部依赖、字节序处理与 pdfTeX 一致（摘要按小端读出）。

/// MD5 填充轮的 64 个正弦表常数（`T[i] = floor(abs(sin(i+1)) × 2^32)`）。
const K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
    0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
    0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
    0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
    0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
    0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
    0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
    0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
    0xeb86d391,
];

/// 各轮的字位移量。
const S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// RFC 1321 §3.4 的四轮非线性函数。
fn f_round(i: usize, x: u32, y: u32, z: u32) -> u32 {
    match i / 16 {
        0 => (x & y) | (!x & z),
        1 => (x & z) | (y & !z),
        2 => x ^ y ^ z,
        _ => y ^ (x | !z),
    }
}

/// 字节串的 MD5 摘要（16 字节，RFC 1321）。
pub fn md5_bytes(data: &[u8]) -> [u8; 16] {
    // 步骤 1：位填充——补 0x80，再补 0 到 ≡56 (mod 64)，最后 8 字节小端位长。
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());

    // 步骤 3：初始化（小端读出即 a0..d0）。
    let mut a0: u32 = 0x6745_2301;
    let mut b0: u32 = 0xefcd_ab89;
    let mut c0: u32 = 0x98ba_dcfe;
    let mut d0: u32 = 0x1032_5476;

    for chunk in msg.chunks_exact(64) {
        let m: [u32; 16] = std::array::from_fn(|i| {
            u32::from_le_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ])
        });
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let fv = f_round(i, b, c, d);
            let g = match i / 16 {
                0 => i,
                1 => (5 * i + 1) % 16,
                2 => (3 * i + 5) % 16,
                _ => (7 * i) % 16,
            };
            let tmp = d;
            d = c;
            c = b;
            let sum = a
                .wrapping_add(fv)
                .wrapping_add(K[i])
                .wrapping_add(m[g]);
            b = b.wrapping_add(sum.rotate_left(S[i]));
            a = tmp;
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut out = [0u8; 16];
    for (i, v) in [a0, b0, c0, d0].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

/// 16 字节摘要 → 32 字符大写 hex。
pub fn md5_hex_upper(data: &[u8]) -> String {
    md5_bytes(data)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect()
}

#[cfg(test)]
mod tests_mdfive {
    use super::*;

    /// RFC 1321 测试套件（A.5）全集——公开域向量逐条核对。
    #[test]
    fn rfc1321_vectors() {
        assert_eq!(
            md5_hex_upper(b""),
            "D41D8CD98F00B204E9800998ECF8427E"
        );
        assert_eq!(
            md5_hex_upper(b"a"),
            "0CC175B9C0F1B6A831C399E269772661"
        );
        assert_eq!(
            md5_hex_upper(b"abc"),
            "900150983CD24FB0D6963F7D28E17F72"
        );
        assert_eq!(
            md5_hex_upper(b"message digest"),
            "F96B697D7CB7938D525A2F31AAF161D0"
        );
        assert_eq!(
            md5_hex_upper(b"abcdefghijklmnopqrstuvwxyz"),
            "C3FCD3D76192E4007DFB496CCA67E13B"
        );
        assert_eq!(
            md5_hex_upper(
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
            ),
            "D174AB98D277D9F5A5611C2C9F419D9F"
        );
        assert_eq!(
            md5_hex_upper(
                b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"
            ),
            "57EDF4A22BE3C955AC49DA2E2107B67A"
        );
    }

    /// 跨填充块（>64 字节）输入：实测向量（python hashlib 对拍）。
    #[test]
    fn multi_block_input() {
        let long = "a".repeat(1000);
        assert_eq!(
            md5_hex_upper(long.as_bytes()),
            "CABE45DCC9AE5B66BA86600CCA6B8BA8"
        );
    }
}
