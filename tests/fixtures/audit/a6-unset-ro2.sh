#!/bin/bash
readonly rv=x
unset rv 2>&1
echo "rc=$?"
unset rv 2>/dev/null
echo "rc2=$?"
