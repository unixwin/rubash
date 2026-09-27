#!/bin/bash
# Area 4 probe: parse.y grammar productions
echo "== function forms =="
f1() { echo f1; }
function f2 { echo f2; }
function f3() { echo f3; }
f1; f2; f3
echo "== function with arg-like parens =="
f4() { echo "n=$#"; }
f4 a b
echo "== coproc =="
coproc COP { echo cop; read -r line; } 2>/dev/null
echo "coprc=$?"
read -r got <&"${COP[0]}" 2>/dev/null || read -r got <"${COP[0]}"
echo "got=$got"
kill "$COP_PID" 2>/dev/null
wait 2>/dev/null
echo "== unnamed coproc =="
coproc { sleep 0.1; } 2>/dev/null; echo "rc=$?"
wait 2>/dev/null
echo "== select (piped answers) =="
printf '2\n' | { select opt in a b c; do echo "picked=$opt"; break; done; }
echo "== select in /dev/null =="
select opt2 in x y; do echo "never"; done < /dev/null
echo "sel-rc=$?"
echo "== case shapes =="
c=es
case $c in
  es) echo matched-es ;;
esac
case $c in
  e?) echo matched-eq ;;
  *) echo star ;;
esac
case $c in
  x) echo x ;;
esac
echo "empty-case-rc=$?"
case abc in
  a*) echo astar ;;&
  *bc) echo bcstar ;;
esac
case abc in
  a*) echo asemi ;&
  *bc) echo bcsemi2 ;;
esac
case $c in
esac
echo "esac-only-rc=$?"
echo "== time pipeline targets =="
time true
time { echo bracket; }
time ( echo subshell )
time echo simple 2>&1
TIMEFORMAT='%R'; time sleep 0
echo "== time -p =="
time -p true
echo "== compound redirects =="
{ echo tofile; } > /tmp/audit_cr.txt 2>/dev/null || { echo tofile; } > cr.txt
cat /tmp/audit_cr.txt 2>/dev/null || cat cr.txt
for i in 1 2; do echo "loop$i"; done > /tmp/audit_cr2.txt 2>/dev/null || for i in 1 2; do echo "loop$i"; done > cr2.txt
cat /tmp/audit_cr2.txt 2>/dev/null || cat cr2.txt
if true; then echo ifbody; fi > /tmp/audit_cr3.txt 2>/dev/null || { if true; then echo ifbody; fi > cr3.txt; }
cat /tmp/audit_cr3.txt 2>/dev/null || cat cr3.txt
echo "== subshell with redirect + pipe =="
(echo inner) | cat
echo "== process substitution =="
cat <(echo psub1) <(echo psub2)
diff <(printf 'a\nb\n') <(printf 'a\nb\n'); echo "rc=$?"
echo "== arithmetic command vs comsub =="
((3+4))
echo "rc-arith=$?"
x=5; ((x+=3)); echo "x=$x"
echo "== nested parens =="
(echo l1; (echo l2))
echo "== until =="
n=0; until [ "$n" -ge 2 ]; do n=$((n+1)); done; echo "n=$n"
echo "== negated pipeline =="
! true; echo "rc=$?"
! false; echo "rc=$?"
echo "== sequential/and-or lists =="
false && echo nope || echo yes
true && echo both && echo chain
echo "== brace group with redirection dup =="
{ echo out; echo err >&2; } 2>&1 1>/dev/null
echo "== empty function body / lone semicolon =="
f5() { ; }; f5; echo "rc=$?"
echo "== let =="
let "v=1+2"; echo "v=$v"
echo "== backtick nesting =="
echo "`echo \\`echo deep\\``"
echo "== multiline string assignment parse =="
long="a
b"
echo "$long" | wc -l
echo DONE
