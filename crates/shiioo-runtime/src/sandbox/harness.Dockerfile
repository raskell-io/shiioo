# The image Shiioo's `container` isolation level runs harnesses in. Built
# on first use by the worker (`docker build -f - …`), tagged with a hash of
# this file, so editing it means a new image.
#
# What is inside: Node (for Claude Code and Codex), git, the GitHub CLI,
# the usual build tools, and the two harnesses from npm. Nothing is
# configured here that belongs to a run: credentials come in at start
# (Codex's auth file mounted read-only, Claude Code's OAuth token from the
# office's secret store), the worktree is mounted at its host path, and
# the home directory is a per-run directory the worker keeps for resumes.
FROM node:22-bookworm-slim
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl git jq ripgrep less procps unzip \
        python3 make g++ pkg-config \
    && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
        -o /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && chmod go+r /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
        > /etc/apt/sources.list.d/github-cli.list \
    && apt-get update && apt-get install -y --no-install-recommends gh \
    && rm -rf /var/lib/apt/lists/*
RUN npm install -g @anthropic-ai/claude-code @openai/codex && npm cache clean --force
# The container has no SSH keys: GitHub remotes go over HTTPS with the
# token `gh` holds (GH_TOKEN in the environment). The worktree's files
# belong to another user id, which git must be told to trust.
RUN git config --system safe.directory '*' \
    && git config --system credential.helper '!gh auth git-credential' \
    && git config --system url."https://github.com/".insteadOf "git@github.com:" \
    && git config --system --add url."https://github.com/".insteadOf "ssh://git@github.com/"
ENV HOME=/root
WORKDIR /root
