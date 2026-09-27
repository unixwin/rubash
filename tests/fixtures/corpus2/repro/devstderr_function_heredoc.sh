u() { cat <<EOF
hi-from-heredoc
EOF
}
u > /dev/stderr
echo "rc=$?"
