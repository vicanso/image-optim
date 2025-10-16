# image-optim

图片压缩服务，支持缩放、裁剪、水印以及图片格式转换功能，并计算压缩之后(同样的尺寸)的图片的差异值。

可以通过环境变量指定以下参数：

- `IMOP_OPENDAL_URL`: OpenDAL 存储的 URL，默认为`file:///opt/images`
- `IMOP_OPTIM_QUALITY`: 图片压缩质量，默认 80
- `IMOP_OPTIM_SPEED`: 图片压缩速度，默认 3

## API 接口说明

基于存储的图片处理服务提供了以下 REST API 接口，所有接口通过 GET 请求并使用 Query 参数传递。

### 1. 图片优化 (`/images/optim`)

对存储中的图片进行压缩优化，可选择转换图片格式。

**请求方式**: `GET /images/optim`

**Query 参数**:
- `file` (必填): 存储中的图片文件路径，最小长度 5 个字符
- `output_type` (可选): 输出图片格式，支持 `jpeg`、`png`、`webp`、`avif`，默认保持原格式
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

### 3. 图片水印 (`/images/watermark`)

为存储中的图片添加水印。

**请求方式**: `GET /images/watermark`

**Query 参数**:
- `file` (必填): 存储中的图片文件路径，最小长度 5 个字符
- `watermark` (必填): 存储中的水印图片路径，最小长度 5 个字符
- `position` (可选): 水印位置，默认为空（具体位置由 imageoptimize 库决定）
- `margin_left` (可选): 水印左边距（像素），默认 0
- `margin_top` (可选): 水印上边距（像素），默认 0
- `quality` (可选): 图片压缩质量，默认值为配置中的 `optim.quality`（默认 80）

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

### 4. 图片裁剪 (`/images/crop`)

按指定区域裁剪图片。

**请求方式**: `GET /images/crop`

**Query 参数**:
- `file` (必填): 存储中的图片文件路径，最小长度 5 个字符
- `x` (可选): 裁剪起始点 X 坐标（像素），默认 0
- `y` (可选): 裁剪起始点 Y 坐标（像素），默认 0
- `width` (必填): 裁剪宽度（像素）
- `height` (必填): 裁剪高度（像素）
- `quality` (可选): 图片压缩质量，默认值为配置中的 `optim.quality`（默认 80）

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

## 配置说明

### 图片优化配置

在配置文件的 `optim` 节中可设置默认参数：

```toml
[optim]
quality = 80  # 默认压缩质量 (0-100)
speed = 3     # 默认压缩速度，主要影响 AVIF 格式 (1-10，速度越快压缩率越低)
```

### 存储配置

图片文件和水印文件均从 OpenDAL 配置的存储中读取，请确保正确配置存储后端。

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

- 所有处理后的图片都会设置 30 天的缓存时间 (`Cache-Control: public, max-age=2592000`)
- 建议在前端或 CDN 层面配置缓存以提高性能

### 错误处理

- 参数验证失败会返回 400 错误
- 文件不存在或读取失败会返回相应的错误信息
- 图片处理失败会返回详细的错误信息，错误类别为 `imageoptimize`
