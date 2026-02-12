#!/usr/bin/env bash
set -euo pipefail

export DEBIAN_FRONTEND=noninteractive
export TZ=Asia/Seoul

apt-get \
    -o Acquire::Retries=3 \
    -o Acquire::https::Timeout=30 \
    update && \
apt-get \
    -o Acquire::Retries=3 \
    -o Acquire::https::Timeout=30 \
    install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    gawk \
    locales \
    less \
    vim \
    curl \
    wget \
    make \
    git \
    python3-dev \
    ca-certificates \
    gnupg \
    lsb-release \
    tzdata \
    zip \
    unzip \
    tar \
    autoconf \
    dh-autoreconf \
    python3.13-venv \
    zlib1g-dev \
    libcurl4-openssl-dev \
    nodejs \
    npm \
    gh \
    rustup \
    jq \
    bc \
    ripgrep \
    procps && \
rm -rf /var/lib/apt/lists/*

update-ca-certificates

ln -s /usr/bin/python3 /usr/bin/python