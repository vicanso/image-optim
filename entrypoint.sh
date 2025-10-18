#!/bin/sh
set -e

case "$1" in
    # 如果 $1 以 - 开头
    -*)
        # 将 "程序" 插入到参数列表的开头
        set -- image-optim "$@"
        ;;
esac

# 执行最终的命令
exec "$@"