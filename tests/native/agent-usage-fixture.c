#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#ifndef AGY_FIXTURE_JSON_PATH
#error "AGY_FIXTURE_JSON_PATH must point to the isolated fixture response"
#endif

#define MAX_FIXTURE_BYTES (32 * 1024)

static int is_usage_request(int argc, char **argv) {
  return argc == 5 && strcmp(argv[1], "-p") == 0 &&
         strcmp(argv[2], "/usage") == 0 &&
         strcmp(argv[3], "--output-format") == 0 &&
         strcmp(argv[4], "json") == 0;
}

static int usage_response(void) {
  FILE *input = fopen(AGY_FIXTURE_JSON_PATH, "rb");
  if (!input) {
    fprintf(stderr, "cannot read isolated Antigravity fixture data\n");
    return 66;
  }

  char buffer[MAX_FIXTURE_BYTES];
  size_t length = fread(buffer, 1, sizeof(buffer), input);
  int too_large = !feof(input);
  int failed = ferror(input);
  fclose(input);
  if (too_large || failed || length == 0) {
    fprintf(stderr, "invalid isolated Antigravity fixture data\n");
    return 65;
  }
  if (fwrite(buffer, 1, length, stdout) != length) return 74;
  if (length == 0 || buffer[length - 1] != '\n') putchar('\n');
  return 0;
}

#if defined(AGY_REAL_EXECUTABLE)
static int delegate_to_real_agy(char **argv) {
  argv[0] = AGY_REAL_EXECUTABLE;
  execv(AGY_REAL_EXECUTABLE, argv);
  fprintf(stderr, "cannot start the verified Antigravity CLI: %s\n",
          strerror(errno));
  return 127;
}
#endif

int main(int argc, char **argv) {
  if (argc == 2 && strcmp(argv[1], "--version") == 0) {
#if defined(AGY_REAL_EXECUTABLE)
    return delegate_to_real_agy(argv);
#else
    puts("Antigravity CLI 1.2.13");
    return 0;
#endif
  }

  if (is_usage_request(argc, argv)) {
#if defined(AGY_REAL_EXECUTABLE)
    return delegate_to_real_agy(argv);
#else
    return usage_response();
#endif
  }

  if (argc == 2 && strcmp(argv[1], "120") == 0) {
    sleep(120);
    return 0;
  }

  fprintf(stderr, "unsupported Antigravity fixture invocation\n");
  return 64;
}
