#!/bin/bash
# 变量和参数扩展测试

echo "=== 变量和参数扩展测试 ==="

# 1. 默认值
unset myvar
echo "1. 默认值: ${myvar:-default_value}"

# 2. 赋值默认值
unset myvar2
echo "2. 赋值默认值: ${myvar2:=assigned_value}"
echo "   变量值: $myvar2"

# 3. 错误提示
unset myvar3
# echo "3. 错误提示: ${myvar3:?variable not set}"

# 4. 替代值
myvar4="set"
echo "4. 替代值: ${myvar4:+alternate_value}"

# 5. 模式删除
filepath="/path/to/file.tar.gz"
echo "5. 删除后缀: ${filepath%.gz}"
echo "   删除前缀: ${filepath#/path}"
echo "   删除最长后缀: ${filepath%%.*}"
echo "   删除最长前缀: ${filepath##*/}"

# 6. 模式替换
text="Hello World World"
echo "6. 替换第一个: ${text/World/Bash}"
echo "   替换所有: ${text//World/Bash}"

# 7. 大小写转换
str="Hello World"
echo "7. 转大写: ${str^^}"
echo "   转小写: ${str,,}"
echo "   首字母大写: ${str^}"

# 8. 间接引用
varname="myvar"
myvar="Hello"
echo "8. 间接引用: ${!varname}"

# 9. 数组长度
arr=(apple banana cherry)
echo "9. 数组长度: ${#arr[@]}"
echo "   元素长度: ${#arr[0]}"

# 10. 子字符串
str="Hello World"
echo "10. 子字符串: ${str:6}"
echo "    子字符串: ${str:0:5}"

echo "=== 变量和参数扩展测试完成 ==="
