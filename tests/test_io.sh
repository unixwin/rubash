#!/bin/bash
# 输入输出和重定向测试

echo "=== 输入输出测试 ==="

# 1. echo 命令
echo "1. echo 命令:"
echo "   普通输出"
echo -n "   不换行输出"
echo ""
echo -e "   转义字符:\tTab"

# 2. printf 命令
echo "2. printf 命令:"
printf "   格式化: %-10s %5d\n" "hello" 42
printf "   十六进制: %x\n" 255
printf "   八进制: %o\n" 255

# 3. Here Document
echo "3. Here Document:"
cat << EOF
   这是here document
   可以多行
   变量替换: $HOME
EOF

# 4. Here Document（不替换）
echo "4. Here Document（不替换）:"
cat << 'EOF'
   这是here document
   $HOME 不会被替换
EOF

# 5. Here String
echo "5. Here String:"
cat <<< "这是here string"

# 6. 文件重定向
echo "6. 文件重定向:"
temp_file="/tmp/test_io_$$.txt"
echo "   写入文件" > "$temp_file"
echo "   追加内容" >> "$temp_file"
cat "$temp_file"
rm -f "$temp_file"

# 7. 错误重定向
echo "7. 错误重定向:"
ls /nonexistent 2>/dev/null
echo "   错误被重定向到/dev/null"

# 8. 标准输出和错误重定向
echo "8. 标准输出和错误重定向:"
ls /tmp /nonexistent > /dev/null 2>&1
echo "   输出和错误都被重定向"

# 9. 管道
echo "9. 管道:"
echo -e "apple\nbanana\ncherry" | sort | head -2

# 10. 命令替换
echo "10. 命令替换:"
echo "   当前目录: $(pwd)"
echo "   文件列表: $(ls /tmp | head -3)"

# 11. 进程替换
echo "11. 进程替换:"
diff <(echo -e "a\nb\nc") <(echo -e "a\nb\nd") || true

# 12. read 命令
echo "12. read 命令:"
echo "input data" | {
    read -r line
    echo "   读取到: $line"
}

# 13. 多行读取
echo "13. 多行读取:"
echo -e "line1\nline2\nline3" | while IFS= read -r line; do
    echo "   $line"
done

echo "=== 输入输出测试完成 ==="
