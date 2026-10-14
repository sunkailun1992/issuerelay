/**
 * `url` with its user-info part (`user:password@`) replaced by `***@`: the rule of core's
 * `env::mask_credentials`, which mask-cases.json holds both implementations to. What stands in
 * front of the first `://` is the scheme and is kept as it is, whatever it holds (a quote, a
 * blank, a `NAME=` prefix), unless it holds an `@` or a `/`: then the `://` belongs to a path or
 * a query, and the value is read from its start, with no scheme (curl also accepts
 * `user:password@host:port` as a proxy). The user-info ends at the last `@` before the first `/`,
 * so a password that holds an `@` is masked whole. Text without such an `@` is returned as it is,
 * and masking a masked value changes nothing.
 *
 * The engine masks the proxy values it reports; the panel masks what it shows once more, so that
 * a value that was not masked on the way is not shown with its password either.
 */
export function maskCredentials(url: string): string {
  const schemeEnd = url.indexOf("://");
  let prefix = "";
  let rest = url;
  if (schemeEnd !== -1) {
    const before = url.slice(0, schemeEnd);
    if (!before.includes("@") && !before.includes("/")) {
      prefix = url.slice(0, schemeEnd + "://".length);
      rest = url.slice(schemeEnd + "://".length);
    }
  }
  const slash = rest.indexOf("/");
  const authority = slash === -1 ? rest : rest.slice(0, slash);
  const at = authority.lastIndexOf("@");
  return at === -1 ? url : `${prefix}***@${rest.slice(at + 1)}`;
}
