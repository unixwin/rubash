mkdir -p gfix
cd gfix
: > 'a b'
shopt -s extglob
echo @('a b')
cd ..
rm -rf gfix
