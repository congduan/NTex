# 随行 Type1（PFB）字形资产

`ntex-pdf` 写出 PDF 时，TFM 8-bit 字体（CM/AMS 等）需要嵌入 Type1 字形
程序，否则 PDF 只写字体名——查看器在本机没有同名字体时用替代字体渲染，
连字缺失、数学符号错位，甚至整段方块。本目录是**无 TeX Live 环境**
（CI、精简部署）唯一可用的 PFB 来源。

查找顺序见 `crates/ntex-pdf/src/type1.rs` 的 `find_pfb`：`NTEX_TYPE1_DIR`
→ `NTEX_TFM_DIR` → 常见 TeX Live 根 → 构建期
`CARGO_MANIFEST_DIR/../../assets/type1`（`cargo test` / 仓库外运行）→
本目录 `assets/type1`（CWD 为仓库根）→ `.local-fonts` → `kpsewhich`。

## 清单（26 件，最小集）

| 用途 | 文件 |
|---|---|
| Transformer 样例实际用字（21 件） | `cmr6` `cmr8` `cmr9` `cmr10` `cmr12` `cmr17` `cmbx8` `cmbx10` `cmbx12` `cmti10` `cmmi5` `cmmi6` `cmmi8` `cmmi9` `cmmi10` `cmmib10` `cmsy8` `cmsy9` `cmsy10` `cmbsy10` `cmex9` |
| AMS 黑板粗体 | `msbm10` |
| TS1 文本符号（cm-super） | `tcrm1095` |
| `ntex-pdf` 回归测试取样 | `cmss12` `cmtt10` `cmex10` |

**为何是最小集而非全家族**：`assets/tfm` 收了 160 个度量，但字形程序单个
~30 KB，全量会让仓库多出数 MB；本目录只覆盖站点自举样例与测试固定取样。
新样张缺字体时日志会逐字打印 `找不到 Type1 字体：<name>.pfb（以不嵌入方式
引用字体）`，按下面「补件」加一行即可。

## 来源（逐字节复制，未改动）

- **25 件（CM 系 24 + `msbm10`）**：TeX Live 2024（TinyTeX）
  `texmf-dist/fonts/type1/public/amsfonts/{cm,cmextra,symbols}/`，按 TeX
  字体名复制。文件头为 AMS 的 2009 年版权声明与 SIL OFL 1.1 授权（见「许可」）。
- **`tcrm1095`**：cm-super 0.3.4（2008-07-09，CTAN
  `fonts/ps-type1/cm-super.zip`）内的 `pfb/sfrm1095.pfb`，sha256
  `3f353abccd39dc18dd1e9356092a70548ddcdc3bd9496e597a4792eeef4dda5e`。
  按 TeX 字体名重命名为 `tcrm1095.pfb`，字节未改动；文件内 `/FontName`
  是 `SFRM1095`（10.95 pt 的 CM Roman）。

  **为什么需要重编码**：官方 `dvips/cm-super-ts1.map` 的对应行是
  `tcrm1095 SFRM1095 "TS1Encoding ReEncodeFont" <cm-super-ts1.enc <sfrm1095.pfb`
  ——该 PFB 内建 `/Encoding StandardEncoding def`，字节本身不带 TS1 槽位布局；
  TS1 的引号、剑标、角度括号等必须按 `cm-super-ts1.enc` 重编码才落到正确
  字形。`ntex-pdf` 写出时对 TS1 族（`tcrm*`/`tcti*`/`tcbx*`/…）补
  `/Encoding /Differences`，表在 `crates/ntex-pdf/src/type1.rs::TS1_ENCODING`。
  （旧版记录误取了同包的 `sftc1095.pfb`——`SFTC1095` 是打字机大小写
  （TCSC），不是 `tcrm*` 对应的罗马体；2026-11-02 已更换。）

校验示例：

```bash
md5sum "/path/to/texmf-dist/fonts/type1/public/amsfonts/cm/cmr10.pfb" \
       assets/type1/cmr10.pfb
sha256sum assets/type1/tcrm1095.pfb
# 3f353abccd39dc18dd1e9356092a70548ddcdc3bd9496e597a4792eeef4dda5e
grep -a -m1 -o '/FontName /[A-Za-z0-9-]*' assets/type1/tcrm1095.pfb  # /FontName /SFRM1095
```

## 许可

本目录 26 件按字体族分别授权，**不能一概写成 Knuth License**：

| 字体族 | 件数 | 许可 | 依据（文件内证据） |
|---|---|---|---|
| CM 系 + `msbm10`（amsfonts） | 25 | **SIL Open Font License 1.1**，含 Reserved Font Name | 每个 PFB 头部：`% Copyright (c) 1997, 2009 American Mathematical Society (<http://www.ams.org>), with Reserved Font Name <字体名>.` 与 `% This Font Software is licensed under the SIL Open Font License, Version 1.1.`；全文见本目录 [`OFL.txt`](OFL.txt)（SIL 官方文本，来源 <https://openfontlicense.org>） |
| `tcrm1095`（cm-super） | 1 | **GPL-2.0-or-later + 字体嵌入例外** | cm-super 0.3.4 `README`：GNU GPL v2「or (at your option) any later version」，并附「As a special exception, permission is granted to include these font programs in a Postscript or PDF file…」；`COPYING` 为 GPL v2 全文 |

两族均允许原样再分发，本目录未修改任何字体字节。注意：这 25 件是 TeX Live
`amsfonts` 发布的 **AMS OFL 版** CM，不是 Knuth 原版（Knuth 原版以 METAFONT
源发行）；旧版 README 将 CM/AMS 统一记为 Knuth License，与文件头证据不符，
已更正。

## 补件

从 TeX Live 复制对应 `.pfb` 到本目录即可（文件名 = TeX 字体名 +
`.pfb`）；若补 AMS 字体，同时确认 `OFL.txt` 在场。

TS1 的 `tcrm*`/`tcti*`/`tcbx*`/`tcss*`/`tctt*` 从 cm-super 取**同族**的
`sfrm*`/`sfti*`/`sfbx*`/…（`sftc*` 是打字机大小写，不是 `tcrm*`！），按
TeX 名重命名；字节内 `/FontName` 保持 cm-super 原值，PDF 侧 `/BaseFont`
由它决定。补完在本目录清单、来源表与许可表登记。
