# Asterisk 22.11.0 built from source (issue #26). Ubuntu 24.04's package is 20.6.0, below the
# 22.7.0 floor where AudioSocket's current protocol lands (issue #4).
FROM ubuntu:24.04

ARG ASTERISK_VERSION=22.11.0
# From downloads.asterisk.org/pub/telephony/asterisk/releases/asterisk-22.11.0.sha256
ARG ASTERISK_SHA256=3bd5ee040509a3d3cd9b1ba9520c18e6ec0a7e7981ca68c457dcd36ba3c54d94

# Build set: the "basic build" and "basic requirements" lists in the tarball's
# contrib/scripts/install_prereq:19-22. wget, bzip2 and patch fetch and patch the bundled
# pjproject (install_prereq:31-33); ca-certificates lets curl and wget use HTTPS.
#
# libssl-dev is on the "addons" list (install_prereq:25), but res_pjsip can't load without it.
# With the bundled pjproject, Asterisk always defines HAVE_PJSIP_TLS_TRANSPORT_RESTART
# (third-party/pjproject/configure.m4:141), so res_pjsip.so calls pjsip_tls_transport_restart.
# pjproject builds that function only with OpenSSL. Without it, loading res_pjsip.so failed
# with "undefined symbol: pjsip_tls_transport_restart". No call uses TLS.
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      build-essential pkg-config autoconf-archive \
      libedit-dev libjansson-dev libsqlite3-dev uuid-dev libxml2-dev libssl-dev \
      wget bzip2 patch curl ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src
RUN curl -fsSLO "https://downloads.asterisk.org/pub/telephony/asterisk/releases/asterisk-${ASTERISK_VERSION}.tar.gz" \
 && echo "${ASTERISK_SHA256}  asterisk-${ASTERISK_VERSION}.tar.gz" | sha256sum -c - \
 && tar xzf "asterisk-${ASTERISK_VERSION}.tar.gz" \
 && rm "asterisk-${ASTERISK_VERSION}.tar.gz"

WORKDIR /usr/src/asterisk-${ASTERISK_VERSION}
# pjproject is bundled by default (configure.ac:473-478). `make menuselect.makeopts` is the
# tarball's own target (Makefile:376). BUILD_NATIVE is off so the binary
# does not depend on the build machine's CPU (docs.asterisk.org, "Building and Installing
# Asterisk"). The AudioSocket modules are not marked defaultenabled=no, so the default
# module selection includes them (inferred from res/res_audiosocket.c and
# apps/app_audiosocket.c).
RUN ./configure \
 && make menuselect.makeopts \
 && menuselect/menuselect --disable BUILD_NATIVE menuselect.makeopts \
 && make -j"$(nproc)" \
 && make install

# No `make samples`: /etc/asterisk holds only what docker-compose.yml mounts from
# asterisk/config, so every line of config in use is one this repo commits, or one
# asterisk/configure.sh generates.

# Asterisk runs as its own unprivileged user (issue #30). The directories it writes at run time
# are handed to that user: the AstDB (registered phones) under /var/lib, logs under /var/log,
# and the control socket that `asterisk -r` connects to under /var/run.
RUN useradd --system --user-group --no-create-home --shell /usr/sbin/nologin asterisk \
 && chown -R asterisk:asterisk \
      /var/lib/asterisk /var/spool/asterisk /var/log/asterisk /var/run/asterisk /var/cache/asterisk
USER asterisk

CMD ["asterisk", "-f", "-vvv"]
