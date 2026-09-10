#!/usr/bin/env bash
# License: MIT (full text: LICENSE at the repository root).
# Copyright 2024-2026 Rust-Shell Contributors.
# 正确运行83个Bash官方测试的脚本
# 使用GNU bash作为对比基准

set -u

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
gnu="D:/Git/bin/bash.exe"
rubash="$repo/target/debug/rubash.exe"
tests_dir="$repo/third_party/bash/tests"
timeout_bin="D:/Git/usr/bin/timeout.exe"
timeout_seconds=15

# 创建工作目录
work_dir="$repo/target/test-run-83"
mkdir -p "$work_dir/bash" "$work_dir/rubash" "$work_dir/results"

# 复制测试文件到工作目录
cp -R "$tests_dir"/. "$work_dir/bash/" 2>/dev/null
cp -R "$tests_dir"/. "$work_dir/rubash/" 2>/dev/null

# 去除CRLF
find "$work_dir/bash" "$work_dir/rubash" -type f -exec sed -i 's/\r$//' {} + 2>/dev/null

# 创建bash wrapper (使用echo避免printf转义问题)
echo '#!/usr/bin/env bash' > "$work_dir/bash/bash"
echo "exec \"$gnu\" \"\$@\"" >> "$work_dir/bash/bash"
chmod +x "$work_dir/bash/bash" 2>/dev/null

echo '#!/usr/bin/env bash' > "$work_dir/rubash/bash"
echo "exec \"$rubash\" \"\$@\"" >> "$work_dir/rubash/bash"
chmod +x "$work_dir/rubash/bash" 2>/dev/null

# 设置THIS_SH环境变量
export THIS_SH="$work_dir/bash/bash"

echo "test\trubash\tbash\tstatus"
echo "---\t------\t----\t------"

pass=0
diff=0
total=0

for src in "$tests_dir"/*.tests; do
  name="$(basename "$src" .tests)"
  total=$((total + 1))
  
  # 运行bash测试
  (cd "$work_dir/bash" && "$timeout_bin" "$timeout_seconds" "$gnu" "./$name.tests" >"$work_dir/results/$name.bash.stdout" 2>"$work_dir/results/$name.bash.stderr")
  brc=$?
  
  # 运行rubash测试
  (cd "$work_dir/rubash" && "$timeout_bin" "$timeout_seconds" "$rubash" "./$name.tests" >"$work_dir/results/$name.rubash.stdout" 2>"$work_dir/results/$name.rubash.stderr")
  rrc=$?
  
  # 标准化行尾
  sed -i 's/\r$//' "$work_dir/results/$name.bash.stdout" "$work_dir/results/$name.rubash.stdout" 2>/dev/null
  
  # 比较
  status="PASS"
  if [[ "$brc" != "$rrc" ]] || ! cmp -s "$work_dir/results/$name.bash.stdout" "$work_dir/results/$name.rubash.stdout"; then
    status="DIFF"
    diff=$((diff + 1))
  else
    pass=$((pass + 1))
  fi
  
  echo "$name\t$rrc\t$brc\t$status"
done

echo ""
echo "TOTAL=$total PASS=$pass DIFF=$diff"
