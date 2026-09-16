using System.Text.Json;
using System.Text.Json.Serialization;
using VedAstro.Library;
using VedAstroSvc.Contracts;
using VedAstroSvc.Muhurta;
using VedAstroSvc.Rules;
using VedAstroSvc.Validate;

var builder = WebApplication.CreateBuilder(args);

// ---- bind: loopback only (CONTRACT.md). Override for tests via VEDASTRO_SVC_URL.
var bindUrl = Environment.GetEnvironmentVariable("VEDASTRO_SVC_URL") ?? "http://127.0.0.1:7793";
builder.WebHost.UseUrls(bindUrl);
builder.Logging.ClearProviders();
builder.Logging.AddSimpleConsole(o => { o.SingleLine = true; o.TimestampFormat = "yyyy-MM-ddTHH:mm:ssZ "; });
builder.Services.ConfigureHttpJsonOptions(o =>
{
    o.SerializerOptions.PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower;
    o.SerializerOptions.DefaultIgnoreCondition = JsonIgnoreCondition.Never;
    o.SerializerOptions.NumberHandling = JsonNumberHandling.AllowNamedFloatingPointLiterals;
});

// ---- canonical config (CONTRACT.md §1): LAHIRI + TRUE nodes; abort if anything else.
Calculate.Ayanamsa = (int)Ayanamsa.LAHIRI;
Calculate.UseMeanRahuKetu = false;
if (Calculate.Ayanamsa != 1 || Calculate.UseMeanRahuKetu) { throw new InvalidOperationException("canonical config violated"); }

// ---- paths
var paths = ServicePaths.Resolve();
var predicates = new PredicateRegistry();
var catalog = new RuleCatalog(paths.XmlDir, predicates);
var evidence = new Evidence(
    Engine: "vedastro-svc",
    Kernel: "VedAstro.Library.Trimmed (upstream master 2026-08-13 + Calculate/HoroscopeCalculatorMethods restored from dc7880af 2023-09-28) / SwissEphNet 2.8.0.2 Moshier fallback",
    Ayanamsa: "LAHIRI",
    TrueNodes: true,
    PositionsRole: "rule-predicates-only; never surfaced as chart positions (blueprint §4 VedAstro)");
var store = new DatasetStore(paths.DatasetDir, paths.DataDir);
var promotions = new PromotionLog(paths.PromotionLogPath);
var finder = new MuhurtaFinder(catalog, predicates, evidence);
var validator = new RuleValidator(catalog, predicates, store, evidence, promotions);

builder.Services.AddSingleton(paths);
builder.Services.AddSingleton(predicates);
builder.Services.AddSingleton(catalog);
builder.Services.AddSingleton(evidence);
builder.Services.AddSingleton(store);
builder.Services.AddSingleton(promotions);
builder.Services.AddSingleton(finder);
builder.Services.AddSingleton(validator);

var app = builder.Build();
var log = app.Logger;

// ---- boot self-test: GOLDEN chart (CONTRACT.md) Moon must be in Swati.
{
    var geo = new GeoLocation("New Delhi", 77.2090, 28.6139);
    var t = new Time(new DateTimeOffset(1990, 3, 15, 6, 30, 0, TimeSpan.Zero).ToOffset(TimeSpan.FromHours(5.5)), geo);
    var moon = Calculate.PlanetNirayanaLongitude(PlanetName.Moon, t).TotalDegrees;
    CacheManager.ResetAll();
    if (moon < 186.0 + 40.0 / 60.0 || moon >= 200.0)
    {
        throw new InvalidOperationException($"GOLDEN self-test failed: Moon sidereal {moon:F4} not in Swati (186.667-200.0)");
    }
    log.LogInformation("golden self-test ok: Moon sidereal {Moon:F4} deg (Swati)", moon);
}
log.LogInformation("rules: proved={Proved} quarantined={Quarantined} xml={Xml}", catalog.CountProved, catalog.CountQuarantined, paths.XmlDir);
{
    // Boot guard: a tampered promotion log is a refused start, not a warning (blueprint §6 fail-closed).
    var chain = promotions.Verify();
    if (!chain.Ok)
    {
        throw new InvalidOperationException($"promotion log {chain.Path} is broken at index {chain.FirstBadIndex}: {chain.Detail}");
    }
    log.LogInformation("promotion log ok: {Entries} entries head={Head} path={Path}", chain.Entries, chain.HeadHash, chain.Path);
}

static IResult Bad(string detail) => Results.BadRequest(new ErrorResponse("bad_request", detail));

app.MapGet("/v1/health", (RuleCatalog cat, DatasetStore ds, PredicateRegistry preds, ServicePaths p, PromotionLog plog) =>
{
    string? datasetError = null;
    try { ds.EnsureLoaded(); } catch (Exception ex) { datasetError = ex.GetType().Name + ": " + ex.Message; }
    var chain = plog.Verify();
    return Results.Ok(new
    {
        ok = datasetError is null && chain.Ok,
        rules_proved = cat.CountProved,
        rules_quarantined = cat.CountQuarantined,
        dataset_rows = ds.PersonRows,
        rules = new
        {
            event_proved = cat.Count(RuleSet.Event, RuleStatus.Proved),
            event_quarantined = cat.Count(RuleSet.Event, RuleStatus.Quarantined),
            horoscope_proved = cat.Count(RuleSet.Horoscope, RuleStatus.Proved),
            horoscope_quarantined = cat.Count(RuleSet.Horoscope, RuleStatus.Quarantined),
            event_proved_with_predicate = cat.CountWithPredicate(RuleSet.Event, RuleStatus.Proved),
            horoscope_proved_with_predicate = cat.CountWithPredicate(RuleSet.Horoscope, RuleStatus.Proved),
            event_predicates_compiled = preds.EventPredicateCount,
            horoscope_predicates_compiled = preds.HoroscopePredicateCount,
            xml_sha256 = cat.FileSha256,
        },
        datasets = new
        {
            persons = ds.PersonRows,
            marriage = ds.MarriageRows,
            persons_sha256 = ds.PersonsSha256,
            marriage_sha256 = ds.MarriageSha256,
            sqlite = ds.DbPath,
            error = datasetError,
        },
        promotion_log = new { path = chain.Path, entries = chain.Entries, chain_ok = chain.Ok, first_bad_index = chain.FirstBadIndex, head_hash = chain.HeadHash },
        ayanamsa = "LAHIRI",
        ayanamsa_swiss_mode = Calculate.Ayanamsa,
        true_nodes = !Calculate.UseMeanRahuKetu,
        engine = "VedAstro.Library.Trimmed",
        swisseph = "SwissEphNet 2.8.0.2 (Moshier fallback, no .se1 files); positions used for rule predicates only",
        bind = bindUrl,
    });
});

app.MapGet("/v1/rules", (RuleCatalog cat, PromotionLog plog, string? set, string? status) =>
{
    RuleSet? rs = null; RuleStatus? st = null;
    if (!string.IsNullOrEmpty(set))
    {
        if (set == "event") { rs = RuleSet.Event; } else if (set == "horoscope") { rs = RuleSet.Horoscope; } else { return Bad("set must be 'event' or 'horoscope'"); }
    }
    if (!string.IsNullOrEmpty(status))
    {
        if (status == "proved") { st = RuleStatus.Proved; } else if (status == "quarantined") { st = RuleStatus.Quarantined; } else { return Bad("status must be 'proved' or 'quarantined'"); }
    }
    // promotion_status is derived from the on-disk log on every call — never cached in memory.
    var promoted = plog.LatestStatusByRule();
    var rules = cat.Query(rs, st).Select(r => new
    {
        id = r.Id,
        name = r.Name,
        description = r.Description,
        tags = r.Tags,
        set = r.Set == RuleSet.Event ? "event" : "horoscope",
        status = r.Status == RuleStatus.Proved ? "proved" : "quarantined",
        nature = r.Nature,
        has_predicate = r.HasPredicate,
        promotion_status = promoted.TryGetValue(r.Id, out var ps) ? ps : PromotionLog.NeverValidatedStatus,
    }).ToList();
    return Results.Ok(new { count = rules.Count, rules });
});

app.MapPost("/v1/muhurta/find", (MuhurtaFindRequest? req, MuhurtaFinder f) =>
{
    if (req is null) { return Bad("JSON body required"); }
    var err = f.Validate(req, out _, out _, out _);
    if (err is not null) { return Bad(err); }
    try
    {
        return Results.Ok(f.Find(req));
    }
    catch (ArgumentException ex) { return Bad(ex.Message); }
});

app.MapPost("/v1/rule/validate", (RuleValidateRequest? req, RuleValidator v, DatasetStore ds) =>
{
    if (req is null) { return Bad("JSON body required"); }
    var err = v.Validate(req, out _);
    if (err is not null) { return Bad(err); }
    try { ds.EnsureLoaded(); }
    catch (Exception ex) { return Results.Json(new ErrorResponse("dataset_unavailable", ex.Message), statusCode: 503); }
    try
    {
        return Results.Ok(v.Run(req));
    }
    catch (ArgumentException ex) { return Bad(ex.Message); }
    catch (PromotionLogException ex) { return Results.Json(new ErrorResponse("promotion_log_unavailable", ex.Message), statusCode: 503); }
});

// Read-only views of the promotion log. There is no route that writes, truncates or deletes it.
app.MapGet("/v1/rule/promotions", (PromotionLog plog, string? rule_id) =>
{
    if (rule_id is not null && (rule_id.Length == 0 || rule_id.Length > 128)) { return Bad("rule_id must be 1-128 chars"); }
    var entries = plog.ReadAll(rule_id);
    return Results.Ok(new { count = entries.Count, path = plog.Path, rule_id, entries });
});

app.MapGet("/v1/rule/promotions/verify", (PromotionLog plog) => Results.Ok(plog.Verify()));

app.Run();

public partial class Program { }

/// <summary>Resolves where the rule XML, the HuggingFace CSVs, the SQLite cache and the promotion log live.</summary>
public sealed record ServicePaths(string XmlDir, string DatasetDir, string DataDir, string PromotionLogPath)
{
    public static ServicePaths Resolve()
    {
        var baseDir = AppContext.BaseDirectory;
        var xml = Environment.GetEnvironmentVariable("VEDASTRO_XML_DIR") ?? Path.Combine(baseDir, "XMLData");
        var repoRoot = FindUp(baseDir, d => Directory.Exists(Path.Combine(d, "vendor", "vedastro", "HuggingFace")));
        var dataset = Environment.GetEnvironmentVariable("VEDASTRO_DATASET_DIR")
                      ?? (repoRoot is null ? Path.Combine(baseDir, "HuggingFace") : Path.Combine(repoRoot, "vendor", "vedastro", "HuggingFace"));
        var svcRoot = FindUp(baseDir, d => File.Exists(Path.Combine(d, "run.sh")) && Directory.Exists(Path.Combine(d, "VedAstroSvc")));
        var data = Environment.GetEnvironmentVariable("VEDASTRO_DATA_DIR")
                   ?? (svcRoot is null ? Path.Combine(baseDir, "data") : Path.Combine(svcRoot, "data"));
        var promo = Environment.GetEnvironmentVariable("VEDASTRO_PROMOTION_LOG");
        if (string.IsNullOrWhiteSpace(promo)) { promo = Path.Combine(data, "rule-promotions.jsonl"); }
        if (!Directory.Exists(xml)) { throw new DirectoryNotFoundException($"rule XML directory not found: {xml} (set VEDASTRO_XML_DIR)"); }
        return new ServicePaths(Path.GetFullPath(xml), Path.GetFullPath(dataset), Path.GetFullPath(data), Path.GetFullPath(promo));
    }

    private static string? FindUp(string start, Func<string, bool> pred)
    {
        var d = new DirectoryInfo(start);
        while (d is not null)
        {
            if (pred(d.FullName)) { return d.FullName; }
            d = d.Parent;
        }
        return null;
    }
}
