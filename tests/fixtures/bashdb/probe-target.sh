#!/usr/bin/env bash
foo() {
  typeset -a items=('a b' 'c')
  echo $((x + 1))
}
x=41
while (( x == 41 )); do
  foo
  (( x++ ))
done
echo done
