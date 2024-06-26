# image-optim

图片压缩服务，支持缩放、裁剪、水印以及图片格式转换功能，并计算压缩之后(同样的尺寸)的图片的差异值。命令格式如下：

- `load`: load=url，通过url加载对应的图片数据
- `resize`: resize=width|height，指定宽度调整图片的尺寸，如果宽或者高设置为0，则表示等比例调整
- `crop`: crop=x|y|width|height，指定参数裁剪
- `watermark`: watermark=url|position|marginLeft|marginTop，指定水印的url获取水印，并添加至指定位置。position如果不指定则为rightBottom，marginLeft与marginTop如果不指定则为0
- `gray`: gray，将图片处理为灰白颜色
- `optim`: optim=format|quality|speed，处理图片压缩转换格式(png, avif, webp, jpeg)，quality如果不指定，则读取env配置(默认为90)，speed如果不指定则读取env配置(默认为3)

注意：avif的处理时间较长，因此如果使用avif格式需要将结果缓存避免每次生成

在服务启动之后，`http://127.0.0.1:3000/pipeline-images/preview`为图片处理预览地址。例如读取`http://127.0.0.1:3013/test.jpeg`的图片并压缩jpeg，处理的url为`http://127.0.0.1:3000/pipeline-images/preview?load=http%3A%2F%2F127.0.0.1%3A3013%2Ftest.jpeg&optim=jpeg%7C90`

响应头中的`X-Dssim-Diff`为压缩后的图片与原图片的差异值(人眼感知，数值*1000)，`X-Ratio`为压缩后的数据与原图片的百分比.

## 指定图片目录

通过`OPTIM_PATH`指定图片目录，`/images/*path`针对此目录中的文件提供图片转换压缩处理。如图片目录下有文件`/asset/original.png`，现希望转换为质量为90的avif，则请求的地址为`/images/asset/original.png_90.avif`

## ENV

默认压缩质量与压缩速度可以通过env指定，具体如下：

- `OPTIM_PATH`: 指定图片处理的目录
- `OPTIM_QUALITY`: 默认压缩质量，如果不指定则为90
- `OPTIM_SPEED`: 默认压缩速度，如果不指定则为5，用于avif压缩(avif压缩较慢，速度选择越高压缩率越低)
- `OPTIM_ALIAS_XXX`: 支持设置参数替换，例如`OPTIM_ALIAS_ABC=http://test.com`表示将参数中的ABC替换为 `http://test.com` ，用于简化图片处理的参数配置
- `OPTIM_DISABLE_DSSIM`: 是否禁用dssim图片对比，如果不需要比对则可禁用(设置为1)

### 压缩图片

- `data`: 可以为http的请求地址或者base64的图片数据
- `data_type`: 若为base64的数据则需指定格式类型，可选
- `output_type`: 图片转换后的格式类型，可选，不指定则不改变
- `quality`: 图片压缩质量
- `speed`: 指定avif的转换速度，设置越高压缩效果越差


```bash
curl -v -XPOST -d '{"data":"https://img2.baidu.com/it/u=3012806272,1276873993&fm=253&fmt=auto&app=138&f=JPEG","output_type":"jpeg","quality":70,"speed":3}' -H 'Content-Type: application/json' 'http://127.0.0.1:3000/optim-images'
```
