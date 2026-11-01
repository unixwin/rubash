/*
 * Loadable-builtin fixture for tests/enable_dynamic_loading.rs.
 *
 * This is a real bash-ABI plugin: it exports `rubash_dynprobe_struct`
 * whose layout is exactly `struct builtin` from bash 5.3 builtins.h:53
 * (name, function, flags, long_doc, short_doc, handle), plus the optional
 * `NAME_builtin_load` / `NAME_builtin_unload` hooks that
 * builtins/enable.def dyn_load_builtin/dyn_unload_builtin look up.
 *
 * The executable passes a `WORD_LIST` (command.h:137) of the command's
 * arguments only: execute_cmd.c:5097 execute_builtin calls
 * `(*builtin) (words->next)`.  The command name is available to a real
 * builtin through this_command_name / make_builtin_argv.
 */

#include <stdio.h>
#include <string.h>

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

/* builtins.h:43-46 */
#define BUILTIN_ENABLED 0x01
#define STATIC_BUILTIN 0x04

static int
rubash_dynprobe_builtin(WORD_LIST *list)
{
  WORD_LIST *l;
  int argc;
  int status;

  status = 0;
  printf("rubash_dynprobe:");
  for (l = list, argc = 0; l; l = l->next, argc++)
    printf(" %s", l->word->word);
  printf(" (argc=%d)\n", argc);
  fflush(stdout);

  /* Let the test observe a non-zero builtin status and the shell.h:71
   * EX_USAGE (258) normalization execute_cmd.c builtin_status applies. */
  for (l = list; l; l = l->next)
    if (strcmp(l->word->word, "fail") == 0)
      status = 7;
    else if (strcmp(l->word->word, "usage") == 0)
      status = 258;
  return status;
}

int
rubash_dynprobe_builtin_load(char *name)
{
  printf("rubash_dynprobe load %s\n", name);
  fflush(stdout);
  return 1;
}

void
rubash_dynprobe_builtin_unload(char *name)
{
  printf("rubash_dynprobe unload %s\n", name);
  fflush(stdout);
}

static char *rubash_dynprobe_doc[] = {
  "Dynamic loading probe builtin.",
  "",
  "Used by rubash's enable -f / enable -d regression tests.",
  (char *)0
};

struct builtin rubash_dynprobe_struct = {
  "rubash_dynprobe",
  rubash_dynprobe_builtin,
  BUILTIN_ENABLED | STATIC_BUILTIN,
  rubash_dynprobe_doc,
  "dynamic loading probe",
  (char *)0
};

/* A second builtin in the same shared object, so the tests can prove
 * GNU's per-handle reference count on enable -d (enable.def:575-583). */
static int
rubash_dynprobe2_builtin(WORD_LIST *list)
{
  WORD_LIST *l;
  int argc;

  printf("rubash_dynprobe2:");
  for (l = list, argc = 0; l; l = l->next, argc++)
    printf(" %s", l->word->word);
  printf(" (argc=%d)\n", argc);
  fflush(stdout);
  return 0;
}

int
rubash_dynprobe2_builtin_load(char *name)
{
  printf("rubash_dynprobe2 load %s\n", name);
  fflush(stdout);
  return 1;
}

void
rubash_dynprobe2_builtin_unload(char *name)
{
  printf("rubash_dynprobe2 unload %s\n", name);
  fflush(stdout);
}

struct builtin rubash_dynprobe2_struct = {
  "rubash_dynprobe2",
  rubash_dynprobe2_builtin,
  BUILTIN_ENABLED | STATIC_BUILTIN,
  rubash_dynprobe_doc,
  "second dynamic loading probe",
  (char *)0
};
