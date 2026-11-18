#!/bin/sh
# Check how a running opsbox panel protects traffic: scheme, TLS version, certificate
# fingerprint, security headers and the attributes of the cookies a login sets.
#
#   scripts/check-transport.sh https://203.0.113.7:8443
#   OPSBOX_USER=admin OPSBOX_PASSWORD=... OPSBOX_CODE=123456 \
#       scripts/check-transport.sh https://panel.example.com
#
# Without OPSBOX_USER/OPSBOX_PASSWORD/OPSBOX_CODE the login step is skipped (the code is
# single use, so the script never guesses one). Exit status: 0 when every check passed,
# 1 when one failed, 2 on bad usage. Compare the fingerprint with the one `opsbox status`
# prints and the one the browser shows for a self-signed certificate.
set -u

usage() {
    echo "usage: $0 <url>   (for example https://203.0.113.7:8443 or http://127.0.0.1:8443)" >&2
    exit 2
}
[ $# -eq 1 ] || usage
url=${1%/}
case $url in
    https://* | http://*) ;;
    *) usage ;;
esac
command -v curl >/dev/null 2>&1 || { echo "curl is required" >&2; exit 2; }

scheme=${url%%://*}
hostport=${url#*://}
hostport=${hostport%%/*}
case $hostport in
    \[*\]*) host=${hostport%%]*}; host=${host#[}; port=${hostport##*]}; port=${port#:} ;;
    *:*) host=${hostport%:*}; port=${hostport##*:} ;;
    *) host=$hostport; port= ;;
esac
[ -n "$port" ] || { [ "$scheme" = https ] && port=443 || port=80; }

fail=0
ok() { printf 'PASS  %s\n' "$1"; }
bad() { printf 'FAIL  %s\n' "$1"; fail=1; }
note() { printf 'INFO  %s\n' "$1"; }

is_loopback() {
    case $1 in 127.* | localhost | ::1) return 0 ;; *) return 1 ;; esac
}

tmp=$(mktemp -d) || exit 2
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

echo "== transport: $url"
note "scheme: $scheme"

# -k: a fresh install uses a self-signed certificate. The fingerprint below is what proves
# it is yours, so print it for comparison instead of failing here.
# SNI is for names; sending an IP address as a server name is invalid.
s_client() {
    case $host in
        *[!0-9.:]*) openssl s_client -connect "$host:$port" -servername "$host" "$@" ;;
        *) openssl s_client -connect "$host:$port" "$@" ;;
    esac
}

if [ "$scheme" = https ]; then
    if command -v openssl >/dev/null 2>&1; then
        # s_client exits non-zero for a self-signed certificate, so judge by what it printed.
        s_client </dev/null >"$tmp/s_client" 2>&1
        if grep -q 'BEGIN CERTIFICATE' "$tmp/s_client"; then
            proto=$(sed -n 's/^ *Protocol *: *//p' "$tmp/s_client" | head -n 1)
            note "TLS version: ${proto:-unknown}"
            case $proto in
                TLSv1.2 | TLSv1.3) ok "TLS 1.2 or newer" ;;
                *) bad "TLS version is not 1.2 or 1.3 (${proto:-unknown})" ;;
            esac
            fp=$(openssl x509 -noout -fingerprint -sha256 <"$tmp/s_client" 2>/dev/null | sed 's/^.*=//')
            if [ -n "$fp" ]; then note "certificate SHA-256: $fp"; else bad "no certificate presented"; fi
        else
            bad "TLS handshake with $host:$port failed"
        fi
        # Anything older than TLS 1.2 must be refused.
        for old in tls1 tls1_1; do
            if s_client "-$old" </dev/null 2>&1 \
                | grep -q 'BEGIN CERTIFICATE'; then
                bad "server accepted legacy protocol $old"
            fi
        done
    else
        note "openssl not found: TLS version and fingerprint not checked"
    fi
    # Plain HTTP on the same port must not work.
    if code=$(curl -s -o /dev/null -m 5 -w '%{http_code}' "http://$hostport/healthz" 2>/dev/null) \
        && [ "$code" != 000 ]; then
        case $code in
            301 | 302 | 307 | 308) ok "plain HTTP on $hostport only redirects ($code)" ;;
            *) bad "plain HTTP on $hostport answered $code" ;;
        esac
    else
        ok "plain HTTP on $hostport is refused"
    fi
else
    if is_loopback "$host"; then
        ok "plain HTTP only on loopback ($host): use an SSH tunnel from your computer"
    else
        bad "plain HTTP on a non-loopback address ($host): passwords and cookies cross the network unencrypted"
    fi
fi

echo "== headers: $url/api/v1/session"
curl -sk -m 10 -D "$tmp/headers" -o /dev/null "$url/api/v1/session" || bad "could not fetch $url/api/v1/session"
for h in x-content-type-options x-frame-options referrer-policy content-security-policy cache-control; do
    v=$(grep -i "^$h:" "$tmp/headers" | head -n 1 | tr -d '\r')
    if [ -n "$v" ]; then ok "$v"; else bad "missing header: $h"; fi
done
v=$(grep -i '^strict-transport-security:' "$tmp/headers" | head -n 1 | tr -d '\r')
if [ -n "$v" ]; then ok "$v"; else note "no Strict-Transport-Security from serve itself (the proxy adds it on the panel domain)"; fi
if grep -iq '^cache-control:.*no-store' "$tmp/headers"; then ok "API answers are not cached"; else bad "API answers are cacheable"; fi

echo "== login cookies"
if [ -n "${OPSBOX_USER:-}" ] && [ -n "${OPSBOX_PASSWORD:-}" ] && [ -n "${OPSBOX_CODE:-}" ]; then
    # Built with printf: the values are never put on a command line that `ps` can show.
    printf '{"username":"%s","password":"%s","code":"%s"}' \
        "$OPSBOX_USER" "$OPSBOX_PASSWORD" "$OPSBOX_CODE" >"$tmp/body"
    status=$(curl -sk -m 15 -D "$tmp/login" -o /dev/null -w '%{http_code}' \
        -H 'Content-Type: application/json' -H "Origin: $url" \
        --data-binary "@$tmp/body" "$url/api/v1/auth/login")
    if [ "$status" = 200 ]; then
        grep -i '^set-cookie:' "$tmp/login" | tr -d '\r' | while IFS= read -r line; do
            printf 'INFO  %s\n' "$(printf '%s' "$line" | sed 's/=[^;]*/=<value>/')"
        done
        session=$(grep -i '^set-cookie:.*session=' "$tmp/login" | head -n 1 | tr -d '\r')
        csrf=$(grep -i '^set-cookie:.*opsbox_csrf=' "$tmp/login" | head -n 1 | tr -d '\r')
        [ -n "$session" ] || bad "login set no session cookie"
        for attr in HttpOnly SameSite=Strict Path=/; do
            printf '%s' "$session" | grep -q "$attr" || bad "session cookie lacks $attr"
        done
        printf '%s' "$session" | grep -qi 'domain=' && bad "session cookie has a Domain attribute"
        if [ "$scheme" = https ]; then
            printf '%s' "$session" | grep -qi '^set-cookie: *__Host-opsbox_session=' \
                || bad "session cookie over HTTPS is not named __Host-opsbox_session"
            printf '%s' "$session" | grep -qi 'secure' || bad "session cookie over HTTPS lacks Secure"
            printf '%s' "$csrf" | grep -qi 'secure' || bad "CSRF cookie over HTTPS lacks Secure"
        else
            printf '%s' "$session" | grep -qi 'secure' && bad "Secure set on a plain-HTTP session"
        fi
        [ "$fail" = 0 ] && ok "session and CSRF cookie attributes"
        # Do not leave the session open.
        sess=$(printf '%s' "$session" | sed 's/^[^:]*: *\([^=]*\)=\([^;]*\).*/\1=\2/')
        csrfv=$(printf '%s' "$csrf" | sed 's/^[^:]*: *[^=]*=\([^;]*\).*/\1/')
        curl -sk -m 10 -o /dev/null -X POST -H "Cookie: $sess; opsbox_csrf=$csrfv" \
            -H "X-Opsbox-CSRF: $csrfv" -H "Origin: $url" "$url/api/v1/auth/logout"
    else
        bad "login answered HTTP $status (check OPSBOX_USER, OPSBOX_PASSWORD and a fresh OPSBOX_CODE)"
    fi
else
    note "login skipped: set OPSBOX_USER, OPSBOX_PASSWORD and OPSBOX_CODE to check cookies"
fi

echo
if [ "$fail" = 0 ]; then echo "RESULT: ok"; else echo "RESULT: FAILED"; fi
exit "$fail"
