import struct
from collections import Counter

def parse_pages(path):
    data = open(path, 'rb').read()
    n = len(data)
    i = n - 1
    while i >= 0 and data[i] == 223:
        i -= 1
    # i points at id byte; 4-byte pointer to postamble is data[i-4:i]
    p = struct.unpack('>I', data[i-4:i])[0]
    assert data[p] == 248, f"expected post (248) at {p}, got {data[p]}"
    # ⚠ DVI 指针语义（tex.web write_postamble/bop 链实测口径）：
    #   - postamble 的 prev_post 字段 = 末页 bop 的**裸地址**（非 +1 伪编码）；
    #   - bop 的 prev-page 指针同样裸地址，首页为 -1（0xFFFFFFFF 有符号）；
    #   - 文件尾 i-4:i 的 postamble 指针才是「值-1」伪编码（此处直读正确）。
    #   postamble 布局：post(1) prev_post(+1) num(+5) den(+9) mag(+13)
    #     page_h(+17) page_w(+21) max_stack(+25) max_pages(+27) first_bop(无此字段)
    pos = struct.unpack('>I', data[p+1:p+5])[0]
    order = []
    while True:
        assert data[pos] == 139, f"expected bop at {pos}, got {data[pos]}"
        counters = data[pos+1:pos+41]
        pos += 41
        prevpage = struct.unpack('>i', data[pos:pos+4])[0]
        pos += 4
        q = pos
        while q < len(data) and data[q] != 140:
            q += 1
        order.append((pos, q, counters))
        # bop 链：prev 指向更早的页，首页（第一页）prev = -1
        if prevpage == -1:
            break
        pos = prevpage
    return data, order

def opname(op):
    if op < 128: return "setchar"
    return {
        132:'setrule', 137:'putrule', 141:'push', 142:'pop',
    }.get(op, f'op{op}')

pages = {}
for name in ['tripos.dvi', 'gt/tripos.dvi']:
    data, pg = parse_pages(name)
    pages[name] = (data, pg)
    print(f"== {name}: {len(pg)} pages, file {len(data)}B")

for idx in range(max(len(pages['tripos.dvi'][1]), len(pages['gt/tripos.dvi'][1]))):
    line = [f"page{idx+1}:"]
    for label, key in [('N', 'tripos.dvi'), ('G', 'gt/tripos.dvi')]:
        da, pg = pages[key]
        if idx < len(pg):
            s, e, counters = pg[idx]
            ops = [opname(o) for o in da[s:e]]
            c = Counter(ops)
            chars = c.get('setchar', 0)
            fonts = sum(v for k, v in c.items() if k.startswith('fnt'))
            line.append(f"{label}: {e-s}B ch={chars} fntops={fonts} push={c.get('push',0)} rules={c.get('setrule',0)+c.get('putrule',0)}")
        else:
            line.append(f"{label}: --")
    print("  " + " | ".join(line))
