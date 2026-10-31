/* odc.h - C ABI for the ODC core library. */
#ifndef ODC_H
#define ODC_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Opaque handle to an opened ODC repository.
 *
 * Threading: a handle may be used by one thread at a time. Sharing a handle
 * between threads requires external synchronisation.
 */
typedef struct OdcRepo OdcRepo;

/*
 * Open an existing repository directory.
 *
 * On success returns a non-NULL handle and stores NULL in *error_message.
 * On failure returns NULL and stores a heap-allocated, NUL-terminated UTF-8
 * error message in *error_message; release it with odc_string_free().
 * error_message may be NULL if the caller does not want the diagnostic.
 */
OdcRepo *odc_repo_open(const char *path_utf8, char **error_message);

/*
 * Close a handle returned by odc_repo_open(). Passing NULL is a no-op.
 */
void odc_repo_close(OdcRepo *repo);

/*
 * Number of committed entries. Returns 0 on failure.
 */
int64_t odc_repo_entry_count(const OdcRepo *repo, char **error_message);

/*
 * Prefix-search normalised keys.
 *
 * limit must be between 1 and 200 inclusive. Returns a NUL-terminated JSON
 * array of {"key": "...", "display": "..."} objects in key order, or NULL on
 * failure. The returned string must be released with odc_string_free().
 */
char *odc_repo_search_json(const OdcRepo *repo, const char *query_utf8,
                           int32_t limit, char **error_message);

/*
 * Look up one entry by exact normalised key and return a NUL-terminated JSON
 * object {"key", "display", "phonetic", "defs", "raw_html", "extra"}.
 *
 * Returns NULL when the key is absent or on failure; *error_message then
 * carries the diagnostic. Release the result with odc_string_free().
 */
char *odc_repo_entry_json(const OdcRepo *repo, const char *key_utf8,
                          char **error_message);

/*
 * Import an MDX file into a standalone .odc file.
 *
 * Returns 0 on success and -1 on failure. On success *report_json_out holds a
 * heap-allocated, NUL-terminated JSON report; release it with
 * odc_string_free(). On failure *report_json_out is NULL and *error_message
 * carries the diagnostic.
 *
 * The output is written atomically: the pack is staged next to the
 * destination and renamed into place. An existing file at out_odc_path is a
 * hard error and is never replaced.
 */
int32_t odc_import_mdx_to_odc(const char *mdx_path, const char *out_odc_path,
                              char **report_json_out, char **error_message);

/*
 * Release a string returned by this library. Passing NULL is a no-op.
 */
void odc_string_free(char *s);

/*
 * Version of the FFI library, e.g. "0.1.0". The pointer is static storage
 * and must not be freed.
 */
const char *odc_version(void);

#ifdef __cplusplus
}
#endif

#endif /* ODC_H */
