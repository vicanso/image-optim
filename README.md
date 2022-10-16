# image-optim

图片压缩服务，支持缩放、裁剪、水印以及图片格式转换功能。命令格式如下：

- `load`: load=url，通过url加载对应的图片数据
- `resize`: resize=width|height，指定宽度调整图片的尺寸，如果宽或者高设置为0，则表示等比例调整
- `crop`: crop=x|y|width|height，指定参数裁剪
- `watermark`: watermark=url|position|marginLeft|marginTop，指定水印的url获取水印，并添加至指定位置。position如果不指定则为rightBottom，marginLeft与marginTop如果不指定则为0
- `optim`: optim=format|quality|speed，处理图片压缩转换格式，quality如果不指定，则读取env配置(默认为90)，speed如果不指定则读取env配置(默认为3)

## ENV

默认压缩质量与压缩速度可以通过env指定，具体如下：

- `OPTIM_QUALITY`: 默认压缩质量，如果不指定则为90
- `OPTIM_SPEED`: 默认压缩速度，如果不指定则为3，用于avif压缩