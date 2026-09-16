# syntax=docker/dockerfile:1.7
# vedastro-svc — VedAstro rule/dataset sidecar, containerized (Phase 8). Build context = repo root.
#
# sdk:10.0 builds Library.Trimmed + VedAstroSvc (Release, framework-dependent); aspnet:10.0 runs it as
# uid 10003. Compose adds: read_only rootfs, tmpfs /tmp, cap_drop ALL, no-new-privileges, internal-only
# network, 1 GiB memory limit. Datasets are bind-mounted read-only at /datasets; the SQLite cache lives
# on the named volume at /data. Binds 0.0.0.0:7793 INSIDE its network namespace only (never published).

FROM mcr.microsoft.com/dotnet/sdk:10.0 AS build
ENV DOTNET_SYSTEM_NET_DISABLEIPV6=1 DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1 \
    DOTNET_SKIP_FIRST_TIME_EXPERIENCE=1
WORKDIR /src
COPY services/vedastro-svc/Library.Trimmed ./Library.Trimmed
COPY services/vedastro-svc/VedAstroSvc ./VedAstroSvc
RUN dotnet restore VedAstroSvc/VedAstroSvc.csproj
RUN dotnet publish VedAstroSvc/VedAstroSvc.csproj -c Release --no-restore -o /app/publish \
    && test -d /app/publish/XMLData && ls /app/publish/XMLData

FROM mcr.microsoft.com/dotnet/aspnet:10.0 AS runtime
# curl only for the container HEALTHCHECK (loopback); the network has no external route anyway.
RUN apt-get update && apt-get install -y --no-install-recommends curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd -g 10003 vedastro \
    && useradd -u 10003 -g 10003 -M -d /nonexistent -s /usr/sbin/nologin vedastro \
    && mkdir -p /data /datasets && chown 10003:10003 /data
WORKDIR /app
COPY --from=build --chown=root:root /app/publish /app
ENV DOTNET_SYSTEM_NET_DISABLEIPV6=1 DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1 \
    DOTNET_EnableDiagnostics=0 DOTNET_gcServer=0 \
    VEDASTRO_SVC_URL=http://0.0.0.0:7793 \
    VEDASTRO_XML_DIR=/app/XMLData \
    VEDASTRO_DATASET_DIR=/datasets \
    VEDASTRO_DATA_DIR=/data \
    HOME=/tmp
USER 10003:10003
EXPOSE 7793
HEALTHCHECK --interval=10s --timeout=5s --start-period=60s --retries=3 \
  CMD ["curl", "-sf", "http://127.0.0.1:7793/v1/health"]
ENTRYPOINT ["dotnet", "/app/VedAstroSvc.dll"]
