FROM archlinux@sha256:ca64cafd0d2e1fd5744d0f607f11a0b33be173790665c9e2931f756ffbb0a37c

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

RUN printf 'Server = https://archive.archlinux.org/repos/2026/07/19/$repo/os/$arch\n' \
      >/etc/pacman.d/mirrorlist \
    && printf '\nXferCommand = /usr/bin/curl --retry 5 --retry-delay 2 --connect-timeout 30 --location --continue-at - --fail --output %%o %%u\n' \
      >>/etc/pacman.conf \
    && pacman -Syu --noconfirm \
    && pacman -S --needed --noconfirm ca-certificates curl diffutils \
    && curl --fail --location --silent --show-error \
      --output /tmp/chromium.pkg.tar.zst \
      https://archive.archlinux.org/packages/c/chromium/chromium-148.0.7778.178-1-x86_64.pkg.tar.zst \
    && curl --fail --location --silent --show-error \
      --output /tmp/ttf-liberation.pkg.tar.zst \
      https://archive.archlinux.org/packages/t/ttf-liberation/ttf-liberation-2.1.5-2-any.pkg.tar.zst \
    && curl --fail --location --silent --show-error \
      --output /tmp/ttf-jetbrains-mono-nerd.pkg.tar.zst \
      https://archive.archlinux.org/packages/t/ttf-jetbrains-mono-nerd/ttf-jetbrains-mono-nerd-3.4.0-2-any.pkg.tar.zst \
    && printf '%s  %s\n' \
      afb93dbb912cf25bfd337a1148f0d5f28652c805e6c191ded6e2976feb9e7a67 /tmp/chromium.pkg.tar.zst \
      cf3c0ae816f6086fc29481e6d82a928d35abde22924fb8cd936f13dd9fe4bd05 /tmp/ttf-liberation.pkg.tar.zst \
      c012dcec4e4d2e1d1db2fe113755e7e75c537f52365c5fca2ca05a08ed71cb50 /tmp/ttf-jetbrains-mono-nerd.pkg.tar.zst \
      | sha256sum --check --strict \
    && pacman -U --noconfirm \
      /tmp/chromium.pkg.tar.zst \
      /tmp/ttf-liberation.pkg.tar.zst \
      /tmp/ttf-jetbrains-mono-nerd.pkg.tar.zst \
    && rm -rf /tmp/*.pkg.tar.zst /var/cache/pacman/pkg/*

WORKDIR /work
