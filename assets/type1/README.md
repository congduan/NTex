# 随行 Type1（PFB）字形资产

`ntex-pdf` 写出 PDF 时，TFM 8-bit 字体（CM/AMS 等）需要嵌入 Type1 字形
程序，否则 PDF 只写字体名——查看器在本机没有同名字体时用替代字体渲染，
连字缺失、数学符号错位，甚至整段方块。本目录是**无 TeX Live 环境**
（CI、精简部署）唯一可用的 PFB 来源。

查找顺序见 `crates/ntex-pdf/src/type1.rs` 的 `find_pfb`：`NTEX_TYPE1_DIR`
→ `NTEX_TFM_DIR` → 常见 TeX Live 根 → 本目录（仓库根）→
`CARGO_MANIFEST_DIR/../../assets/type1`（`cargo test`）→ `.local-fonts` →
`kpsewhich`。

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

- CM 系 24 件 + `msbm10`：TeX Live 2024（TinyTeX）
  `texmf-dist/fonts/type1/public/amsfonts/{cm,cmextra,symbols}/`；
- `tcrm1095`：cm-super 发行包的 `pfb/sftc1095.pfb`。**文件名按 TeX 字体名
  存放**（查找链以 `tcrm1095.pfb` 为键），字节内 `/FontName` 仍是
  `SFTC1095`——cm-super 的 TC 族与 `jknappen/ec` 的 `tcrm*` 度量同源、
  编码同为 TS1，PDF 侧 `/BaseFont` 由字节内 `/FontName` 决定，故可直嵌。

校验示例：

```bash
md5sum "/path/to/texmf-dist/fonts/type1/public/amsfonts/cm/cmr10.pfb" \
       assets/type1/cmr10.pfb
grep -a -m1 -o '/FontName /[A-Za-z0-9-]*' assets/type1/tcrm1095.pfb  # /FontName /SFTC1095
```

## 许可

Computer Modern / AMS 字体按 AMS 的 Computer Modern 字体发布条款
（Knuth License，LPPL 兼容）随 TeX Live 再分发；cm-super 字体为 GPL。
两者均允许原样再分发，本目录未做任何修改。

## 补件

从 TeX Live 复制对应 `.pfb` 到本目录即可（文件名 = TeX 字体名 +
`.pfb`）；若是 TS1 的 `tcrm*`/`tcss*`/`tctt*`，从 cm-super 取同名尺寸的
`sftc*.pfb` 并按 TeX 名重命名。补完在本目录清单与来源表中登记。
