/*
 * Loadable-builtin fixture for tests/issue3605_globignore_strmatch_regressions.rs.
 *
 * Mirrors glob-bracket.tests' strmatch.so: a bash-ABI plugin that calls the
 * host's exported `strmatch()` back into the shell image (lib/glob/strmatch.c
 * :32-39). The second argument is the pattern, the third the name, and the
 * optional fourth an FNM_* flag word (default FNM_PATHNAME). The builtin
 * prints GNU's return value, 0 = match / 1 = no match.
 */

#include <stdio.h>
#include <stdlib.h>

int strmatch(const char *, const char *, int);

typedef struct word_desc {
  char *word;
  int flags;
} WORD_DESC;

typedef struct word_list {
  struct word_list *next;
  WORD_DESC *word;
} WORD_LIST;

typedef struct builtin {
  char *name;
  int (*function)(WORD_LIST *);
  int flags;
  char *const *long_doc;
  const char *short_doc;
  char *handle;
} BUILTIN;

#define BUILTIN_ENABLED 0x01

static int
strmatch_probe_builtin(WORD_LIST *list)
{
  WORD_LIST *l;
  char *args[4];
  int argc;
  int flags;

  for (l = list, argc = 0; l && argc < 4; l = l->next, argc++)
    args[argc] = l->word->word;
  if (argc < 2) {
    printf("strmatch_probe: usage: pattern name [flags]\n");
    return 2;
  }
  flags = argc >= 3 ? atoi(args[2]) : 1 /* FNM_PATHNAME */;
  printf("strmatch=%d\n", strmatch(args[0], args[1], flags));
  fflush(stdout);
  return 0;
}

static char *strmatch_probe_doc[] = {
  "Call the host image's exported strmatch(pattern, name, flags).",
  (char *)0
};

struct builtin strmatch_probe_struct = {
  "strmatch_probe",
  strmatch_probe_builtin,
  BUILTIN_ENABLED,
  strmatch_probe_doc,
  "strmatch_probe pattern name [flags]",
  (char *)0
};
