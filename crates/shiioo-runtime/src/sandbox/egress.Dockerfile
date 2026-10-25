# The egress sidecar for Shiioo's container runs: tinyproxy with an
# allow list, mounted at /etc/tinyproxy by the worker. Runs as the
# unprivileged user the package creates.
FROM alpine:3.20
RUN apk add --no-cache tinyproxy
USER tinyproxy
EXPOSE 3128
CMD ["tinyproxy", "-d", "-c", "/etc/tinyproxy/tinyproxy.conf"]
