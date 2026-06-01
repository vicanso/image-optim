# image-optim

图片压缩服务，支持缩放、裁剪、水印以及图片格式转换功能，并计算压缩之后(同样的尺寸)的图片的差异值。

可以通过环境变量指定以下参数：

- `IMOP_OPENDAL_URL`: 默认 OpenDAL 存储的 URL，默认为`file:///opt/images`，支持以下格式：
  - 本地文件系统：`file:///opt/images`
  - HTTP/HTTPS：`http://your-server/images` 或 `https://your-server/images`
  - S3 兼容存储：`s3://bucket-name?region=us-east-1&access_key_id=xxx&secret_access_key=xxx&endpoint=https://s3.amazonaws.com`
- `IMOP_OPENDAL_<NAME>_URL`: 命名 OpenDAL 存储源（可配置多个），名字大小写不敏感。请求时通过 `?source=<NAME>` 选择对应源；不传 `source` 时使用默认存储。例如：
  - `IMOP_OPENDAL_USERS_URL=file:///opt/images/users`
  - `IMOP_OPENDAL_BAIDU_URL=https://www.baidu.com`
  - 调用：`/images/optim?source=baidu&file=image.jpg` 从 `baidu` 源读取 `https://www.baidu.com/image.jpg`
- `IMOP_OPTIM_QUALITY`: 图片压缩质量（全格式默认值），默认 80
- `IMOP_OPTIM_QUALITY_JPEG`: JPEG 格式的压缩质量，未设置时使用 `IMOP_OPTIM_QUALITY`
- `IMOP_OPTIM_QUALITY_PNG`: PNG 格式的压缩质量，未设置时使用 `IMOP_OPTIM_QUALITY`
- `IMOP_OPTIM_QUALITY_WEBP`: WebP 格式的压缩质量，未设置时使用 `IMOP_OPTIM_QUALITY`
- `IMOP_OPTIM_QUALITY_AVIF`: AVIF 格式的压缩质量，未设置时使用 `IMOP_OPTIM_QUALITY`
- `IMOP_OPTIM_SPEED`: 图片压缩速度，默认 3
- `IMOP_PRESET_<NAME>`: 命名预设（可配置多个），名字大小写不敏感。值格式：`<op>&<key>=<value>&...`，其中 `<op>` ∈ `optim` / `resize` / `fit` / `watermark` / `crop` / `padding`。例如：
  - `IMOP_PRESET_THUMB=fit&width=300&quality=70`
  - `IMOP_PRESET_LOGOSTRIP=optim&strip=true`
  - 调用：`/images/preset?preset=thumb&file=photo.jpg`，请求侧参数可覆盖预设默认值（如 `&quality=90`）

```bash
docker run -d \
  --name image-optim \
  -p 3000:3000 \
  -v ~/Downloads:/opt/images \
  -e IMOP_OPENDAL_URL=file:///opt/images \
  -e IMOP_OPTIM_QUALITY=80 \
  -e IMOP_OPTIM_QUALITY_JPEG=85 \
  -e IMOP_OPTIM_QUALITY_PNG=90 \
  -e IMOP_OPTIM_QUALITY_WEBP=80 \
  -e IMOP_OPTIM_QUALITY_AVIF=70 \
  -e IMOP_OPTIM_SPEED=3 \
  vicanso/image-optim
```

## API 接口说明

基于存储的图片处理服务提供了以下 REST API 接口，所有接口通过 GET 请求并使用 Query 参数传递。

### 通用参数

所有图片处理 GET 接口（`optim` / `resize` / `fit` / `watermark` / `crop` / `padding` / `preset`）都支持以下通用参数：

| 参数 | 说明 | 示例 |
|------|------|------|
| `source` | 选择命名 OpenDAL 存储源（对应 `IMOP_OPENDAL_<NAME>_URL`），名字大小写不敏感；未提供时使用默认 `IMOP_OPENDAL_URL`。`watermark` 端点的水印文件也从同一源读取 | `source=users` |
| `output_type` | 输出格式：`jpeg` `png` `webp` `avif` `jxl` `auto`；`auto` 会根据请求的 `Accept` 头协商。⚠️ `jxl` 会丢弃 alpha 通道（libjxl 0.11 / jpegxl-rs 0.11 限制） | `output_type=webp` |
| `quality` | 压缩质量 0-100，默认取配置 `optim.quality_<format>` 或 `optim.quality` | `quality=85` |
| `rotate` | 旋转角度：`90` / `180` / `270` | `rotate=90` |
| `flip` | 翻转方向：`h` / `horizontal` 或 `v` / `vertical` | `flip=h` |
| `gray` | 转灰度图 | `gray=true` |
| `sharpen` | USM 锐化，格式 `sigma` 或 `sigma,threshold` | `sharpen=1.0` |
| `blur` | 高斯模糊 sigma | `blur=2.0` |
| `brighten` | 亮度调整，正数增亮、负数减暗 | `brighten=20` |
| `contrast` | 对比度，浮点字符串 | `contrast=1.5` |
| `strip` | 剥离 EXIF 元数据（不重新编码） | `strip=true` |
| `padding_width` | 画布扩展宽度（像素），与 `padding_height` 配合使用 | `padding_width=1000` |
| `padding_height` | 画布扩展高度（像素） | `padding_height=1000` |
| `padding_color` | 画布填充色，十六进制（如 `#ffffff` 或 `#ffffff80`），默认透明 | `padding_color=%23ffffff` |

> 运行时也可通过 `GET /images/command` 获取完整的 Markdown API 文档（包含上述所有参数及示例）。

### 1. 图片优化 (`/images/optim`)

对存储中的图片进行压缩优化，可选择转换图片格式。

**请求方式**: `GET /images/optim`

**Query 参数**:
- `file` (必填): 存储中的图片文件路径，最小长度 5 个字符
- `output_type` (可选): 输出图片格式，支持 `jpeg`、`png`、`webp`、`avif`、`jxl`、`auto`，默认保持原格式（`jxl` 会丢弃 alpha）
- `quality` (可选): 图片压缩质量，范围 0-100，默认值为配置中的 `optim.quality`（默认 80）

**返回头部**:
- `Content-Type`: 对应的图片 MIME 类型
- `Cache-Control`: `public, max-age=2592000` (30天缓存)
- `X-Dssim-Diff`: 压缩后与原图的差异值（人眼感知差异）
- `X-Ratio`: 压缩率百分比

**示例**:
```bash
# 优化图片为 webp 格式，质量 75
curl "http://127.0.0.1:3000/images/optim?file=images/photo.jpg&output_type=webp&quality=75"

# 优化图片保持原格式
curl "http://127.0.0.1:3000/images/optim?file=images/photo.png"
```

---

### 2. 图片缩放 (`/images/resize`)

调整存储中图片的尺寸，支持等比例缩放。

**请求方式**: `GET /images/resize`

**Query 参数**:
- `file` (必填): 存储中的图片文件路径，最小长度 5 个字符
- `width` (可选): 目标宽度（像素），默认 0
- `height` (可选): 目标高度（像素），默认 0
- `quality` (可选): 图片压缩质量，默认值为配置中的 `optim.quality`（默认 80）
- `output_type` (可选): 输出图片格式，支持 `jpeg`、`png`、`webp`、`avif`、`jxl`、`auto`，默认保持原格式（`jxl` 会丢弃 alpha）

**注意事项**:
- `width` 和 `height` 不能同时为 0
- 当 `width` 为 0 时，根据 `height` 等比例计算宽度
- 当 `height` 为 0 时，根据 `width` 等比例计算高度
- 缩放后会自动进行图片优化处理

**示例**:
```bash
# 缩放图片宽度为 800px，高度等比例调整
curl "http://127.0.0.1:3000/images/resize?file=images/photo.jpg&width=800"

# 缩放图片到指定尺寸 1024x768
curl "http://127.0.0.1:3000/images/resize?file=images/photo.jpg&width=1024&height=768&quality=85"
```

---

### 3. 适应缩放 (`/images/fit`)

缩放至不超过给定的 `width` × `height` 边界，保持原始宽高比，不放大原图。与 `resize` 的关键区别：若原图已在边界内则不做任何缩放。

**请求方式**: `GET /images/fit`

**Query 参数**:
- `file` (必填): 存储中的图片文件路径，最小长度 5 个字符
- `width` (可选): 边界宽度（像素），默认 0
- `height` (可选): 边界高度（像素），默认 0
- `quality` (可选): 图片压缩质量，默认值为配置中的 `optim.quality`（默认 80）
- `output_type` (可选): 输出图片格式，支持 `jpeg`、`png`、`webp`、`avif`、`jxl`、`auto`，默认保持原格式（`jxl` 会丢弃 alpha）

**注意事项**:
- `width` 和 `height` 不能同时为 0
- 仅在原图任一边超过边界时才会缩放
- 缩放后会自动进行图片优化处理

**示例**:
```bash
# 在 800x600 边界内适应缩放
curl "http://127.0.0.1:3000/images/fit?file=images/photo.jpg&width=800&height=600"
```

---

### 4. 图片水印 (`/images/watermark`)

为存储中的图片添加水印。

**请求方式**: `GET /images/watermark`

**Query 参数**:
- `file` (必填): 存储中的图片文件路径，最小长度 5 个字符
- `watermark` (必填): 存储中的水印图片路径，最小长度 5 个字符
- `position` (可选): 水印位置，默认为空（具体位置由 imageoptimize 库决定）
- `margin_left` (可选): 水印左边距（像素），默认 0
- `margin_top` (可选): 水印上边距（像素），默认 0
- `quality` (可选): 图片压缩质量，默认值为配置中的 `optim.quality`（默认 80）
- `output_type` (可选): 输出图片格式，支持 `jpeg`、`png`、`webp`、`avif`、`jxl`、`auto`，默认保持原格式（`jxl` 会丢弃 alpha）

**说明**:
- 水印图片会被 Base64 编码后传递给图片处理库
- 添加水印后会自动进行图片优化处理

**示例**:
```bash
# 添加水印到右下角
curl "http://127.0.0.1:3000/images/watermark?file=images/photo.jpg&watermark=watermarks/logo.png&position=rightBottom"

# 添加水印并指定边距
curl "http://127.0.0.1:3000/images/watermark?file=images/photo.jpg&watermark=watermarks/logo.png&margin_left=20&margin_top=20&quality=90"
```

---

### 5. 图片裁剪 (`/images/crop`)

按指定区域裁剪图片。

**请求方式**: `GET /images/crop`

**Query 参数**:
- `file` (必填): 存储中的图片文件路径，最小长度 5 个字符
- `x` (可选): 裁剪起始点 X 坐标（像素），默认 0
- `y` (可选): 裁剪起始点 Y 坐标（像素），默认 0
- `width` (必填): 裁剪宽度（像素）
- `height` (必填): 裁剪高度（像素）
- `quality` (可选): 图片压缩质量，默认值为配置中的 `optim.quality`（默认 80）
- `output_type` (可选): 输出图片格式，支持 `jpeg`、`png`、`webp`、`avif`、`jxl`、`auto`，默认保持原格式（`jxl` 会丢弃 alpha）

**说明**:
- 裁剪后会自动进行图片优化处理
- 坐标从图片左上角 (0, 0) 开始

**示例**:
```bash
# 从 (100, 100) 位置裁剪 500x500 的区域
curl "http://127.0.0.1:3000/images/crop?file=images/photo.jpg&x=100&y=100&width=500&height=500"

# 从左上角裁剪 800x600 的区域
curl "http://127.0.0.1:3000/images/crop?file=images/photo.jpg&width=800&height=600&quality=85"
```

---

### 6. 画布填充 (`/images/padding`)

将原图居中放置，并将画布扩展至指定尺寸，扩展区域填充指定颜色（默认透明）。

**请求方式**: `GET /images/padding`

**Query 参数**:
- `file` (必填): 存储中的图片文件路径，最小长度 5 个字符
- `width` (必填): 目标画布宽度（像素）
- `height` (必填): 目标画布高度（像素）
- `color` (可选): 填充色，十六进制字符串（如 `#ffffff` 或 `#ffffff80`，URL 中 `#` 需编码为 `%23`），默认透明
- `quality` (可选): 图片压缩质量，默认值为配置中的 `optim.quality`（默认 80）
- `output_type` (可选): 输出图片格式，支持 `jpeg`、`png`、`webp`、`avif`、`jxl`、`auto`，默认保持原格式（`jxl` 会丢弃 alpha）

**示例**:
```bash
# 扩展画布到 1000x1000，白底
curl "http://127.0.0.1:3000/images/padding?file=images/photo.jpg&width=1000&height=1000&color=%23ffffff"

# 扩展画布并输出为 webp
curl "http://127.0.0.1:3000/images/padding?file=images/photo.jpg&width=1200&height=800&output_type=webp"
```

---

### 7. 流水线处理 (`/images/process`)

以 JSON 方式提交 Base64 编码的图片数据，按 `ops` 数组顺序执行任意组合操作，返回 Base64 编码的处理结果。适用于不依赖 OpenDAL 存储的临时处理场景。

**请求方式**: `POST /images/process`

**请求头**: `Content-Type: application/json`

**请求体字段**:
- `data` (必填): Base64 编码的原始图片二进制
- `ext` (必填): 原始图片格式扩展名（如 `jpg`、`png`、`webp`、`avif`）
- `ops` (可选): 操作数组；若不包含 `optim`，最后会自动追加一次默认编码

**支持的 `ops` 操作类型**（`type` 字段）:

| type | 参数 |
|------|------|
| `fit` | `width`, `height` |
| `resize` | `width`, `height` |
| `crop` | `x`, `y`, `width`, `height` |
| `rotate` | `deg`（90 / 180 / 270） |
| `flip` | `dir`（h / v） |
| `gray` | — |
| `sharpen` | `sigma`, `threshold`（默认 0） |
| `blur` | `sigma` |
| `brighten` | `value` |
| `contrast` | `value` |
| `strip` | — |
| `padding` | `width`, `height`, `color`（默认透明） |
| `watermark` | `data`（Base64 水印图）, `position`, `margin_left`, `margin_top` |
| `optim` | `output_type`, `quality` |

**响应字段**:
- `data`: Base64 编码的处理后图片
- `ext`: 实际输出格式扩展名
- `ratio`: 压缩率百分比（处理后 / 原始大小，最低 1）
- `diff`: DSSIM 差异值（仅纯优化场景有意义）

**示例**:
```bash
curl -X POST http://127.0.0.1:3000/images/process \
  -H "Content-Type: application/json" \
  -d '{
    "data": "<base64-image>",
    "ext": "jpg",
    "ops": [
      {"type": "fit", "width": 800, "height": 600},
      {"type": "sharpen", "sigma": 1.0},
      {"type": "optim", "output_type": "webp", "quality": 80}
    ]
  }'
```

---

### 8. 命令文档 (`/images/command`)

返回完整的 Markdown 格式 API 文档，可在运行时查阅最新接口与参数说明。

**请求方式**: `GET /images/command`

**响应**: `text/plain` 格式的 Markdown 文本。

**示例**:
```bash
curl http://127.0.0.1:3000/images/command
```

---

### 9. 预设处理 (`/images/preset`)

通过环境变量 `IMOP_PRESET_<NAME>` 预先定义"操作 + 参数"组合，请求侧只需选择预设名 + 文件路径即可，便于前端/CDN 规范化 URL（同一份缩略图永远同一个 URL）。

**预设格式**: `IMOP_PRESET_<NAME>=<op>&<key>=<value>&...`
- `<op>` ∈ `optim` / `resize` / `fit` / `watermark` / `crop` / `padding`
- 名字大小写不敏感
- 启动时无效预设会记录 warn 并跳过，不阻断启动

**请求方式**: `GET /images/preset`

**Query 参数**:
- `preset` (必填): 预设名（对应 `IMOP_PRESET_<NAME>`），大小写不敏感
- `file` (必填): 存储中的图片文件路径，最小长度 5 个字符
- 其余参数 (可选): 任意通用参数或预设字段。**请求侧值会覆盖预设默认值**

**示例**:
```bash
# 启动时配置
export IMOP_PRESET_THUMB="fit&width=300&quality=70"
export IMOP_PRESET_LOGOSTRIP="optim&strip=true"
export IMOP_PRESET_SQUARE="padding&width=1000&height=1000&color=%23ffffff"

# 调用：使用预设默认值
curl "http://127.0.0.1:3000/images/preset?preset=thumb&file=images/photo.jpg"

# 调用：覆盖预设的 quality
curl "http://127.0.0.1:3000/images/preset?preset=thumb&file=images/photo.jpg&quality=90"

# 调用：从命名存储源读取
curl "http://127.0.0.1:3000/images/preset?preset=thumb&file=photo.jpg&source=users"
```

---

## 配置说明

### 图片优化配置

在配置文件的 `optim` 节中可设置默认参数：

```toml
[optim]
quality = 80        # 全格式默认压缩质量 (0-100)
quality_jpeg = 85   # JPEG 专用质量，未设置时使用 quality
quality_png = 90    # PNG 专用质量，未设置时使用 quality
quality_webp = 80   # WebP 专用质量，未设置时使用 quality
quality_avif = 70   # AVIF 专用质量，未设置时使用 quality
speed = 3           # 默认压缩速度，主要影响 AVIF 格式 (1-10，速度越快压缩率越低)
max_age = "1440h"   # 默认缓存时间，默认 60 天(只能使用h的表示方式，不能使用d)
auto_output_types = ["avif", "png"] # 自动检测时使用的图片格式，用于accept头部的图片格式检测
```

### 存储配置

图片文件和水印文件均从 OpenDAL 配置的存储中读取，支持本地文件系统、HTTP/HTTPS 以及 S3 兼容存储（阿里云 OSS、MinIO 等），通过 `IMOP_OPENDAL_URL` 环境变量配置。

---

## 技术细节

### 图片处理流程

所有图片处理接口遵循以下流程：

1. **参数验证**: 使用 `validator` crate 验证输入参数
2. **加载图片**: 从 OpenDAL 存储中读取原始图片数据
3. **图片处理**: 使用 `imageoptimize` 库执行相应的处理操作（裁剪、缩放、水印等）
4. **格式优化**: 根据指定的质量和速度参数进行压缩优化
5. **计算差异**: 对于 `optim` 接口，会计算处理后图片与原图的差异值（DSSIM）
6. **返回结果**: 返回处理后的图片数据及相关元数据（差异值、压缩率等）

### 缓存策略

- 所有处理后的图片都会设置缓存时间 (`Cache-Control: public, max-age=2592000`)，若图片格式是基于`accpet`判断，则缓存为`private`
- 建议在反向代理或 CDN 层面配置缓存以提高性能

### 错误处理

- 参数验证失败会返回 400 错误
- 文件不存在或读取失败会返回相应的错误信息
- 图片处理失败会返回详细的错误信息，错误类别为 `imageoptimize`
