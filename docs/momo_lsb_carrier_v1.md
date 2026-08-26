# MOMO LSB Carrier v1

**状态：** 0.5.0 profile

MOMO LSB Carrier v1 是 MOMO 自有的角色图片载体扩展，不属于 Character Card、CHARX、
PNG 或 JPEG 标准。它把一个载荷写入同一张图片解码后像素的 RGB 通道最低有效位；
alpha 通道不参与编码，也不在文件尾追加第二个文件或隐藏归档。

## 像素顺序与容量

图片先由正常图片解码器转换为从左到右、从上到下的 8-bit interleaved RGB 或 RGBA
采样。每个像素依次使用 R、G、B 的 1 个最低有效位，字节按最高位到最低位写入。
容量是 `floor(width * height * 3 / 8)` 字节，包含 24 字节头。容量不足必须拒绝，且
不得自动增加位深、修改 alpha、缩放图片或降低载荷完整性。

## 24 字节大端序头

| Offset | Bytes | 含义 |
| --- | ---: | --- |
| 0 | 8 | ASCII magic `MOMOLSB1` |
| 8 | 1 | version，固定为 `1` |
| 9 | 1 | flags；bit 0 表示 zstd，其他位必须为 0 |
| 10 | 1 | payload type：1=基线角色数据，2=MOC，3=CHARX |
| 11 | 1 | reserved，写 0 |
| 12 | 4 | stored payload length |
| 16 | 4 | original payload length |
| 20 | 4 | 解压后载荷的 CRC32 |

基线推荐 payload type 1，并使用 zstd 压缩。MOC 或 CHARX 只有在调用方明确选择且图片
容量足够时才可嵌入。实现必须限制声明长度和解压后大小，先校验完整性再解析角色数据。

## 可移植性边界

载体只依赖解码后的像素样本，因此删除 IEND/EOI 后无关尾部、删除元数据、重排 PNG
chunk，或换成能逐样本保真的无损容器后仍可解码。任何缩放、裁剪、滤镜、颜色空间
量化、有损重压缩或像素清洗都可能破坏载体。PNG chunk、标准 CHARX、CHARX-JPEG 与
既有导入能力继续作为兼容路径；LSB 不冒充这些格式，也不替代文件型交换格式。
