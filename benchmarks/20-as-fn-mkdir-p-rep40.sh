#!/usr/bin/env bash
# Perf probe 20: as_fn_mkdir_p 44-line autoconf unit repeated 40 times
# (definitions only, never called). The #178 rep-40 amplifier shape,
# extracted verbatim from GNU bash 5.3 configure lines 325-368.
# PERF: runs=5 timeout=60
as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

as_fn_mkdir_p ()
{

  case $as_dir in #(
  -*) as_dir=./$as_dir;;
  esac
  test -d "$as_dir" || eval $as_mkdir_p || {
    as_dirs=
    while :; do
      case $as_dir in #(
      *\'*) as_qdir=`printf "%s\n" "$as_dir" | sed "s/'/'\\\\\\\\''/g"`;; #'(
      *) as_qdir=$as_dir;;
      esac
      as_dirs="'$as_qdir' $as_dirs"
      as_dir=`$as_dirname -- "$as_dir" ||
$as_expr X"$as_dir" : 'X\(.*[^/]\)//*[^/][^/]*/*$' \| \
	 X"$as_dir" : 'X\(//\)[^/]' \| \
	 X"$as_dir" : 'X\(//\)$' \| \
	 X"$as_dir" : 'X\(/\)' \| . 2>/dev/null ||
printf "%s\n" X"$as_dir" |
    sed '/^X\(.*[^/]\)\/\/*[^/][^/]*\/*$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)[^/].*/{
	    s//\1/
	    q
	  }
	  /^X\(\/\/\)$/{
	    s//\1/
	    q
	  }
	  /^X\(\/\).*/{
	    s//\1/
	    q
	  }
	  s/.*/./; q'`
      test -d "$as_dir" && break
    done
    test -z "$as_dirs" || eval "mkdir $as_dirs"
  } || test -d "$as_dir" || as_fn_error $? "cannot create directory $as_dir"


} # as_fn_mkdir_p

exit 0
