using System.Net.Http.Json;
using System.Text.Json;
using System.Xml.Linq;
using Microsoft.AspNetCore.Mvc.Testing;
using VedAstroSvc.Contracts;
using VedAstroSvc.Muhurta;
using VedAstroSvc.Rules;
using VedAstroSvc.Validate;
using Xunit;

namespace VedAstroSvc.Tests;

public sealed class Fixture : IDisposable
{
    public PredicateRegistry Predicates { get; }
    public RuleCatalog Catalog { get; }
    public DatasetStore Store { get; }
    public Evidence Evidence { get; } = new("test", "test", "LAHIRI", true, "predicates-only");
    public string XmlDir { get; }
    public string DatasetDir { get; }

    public Fixture()
    {
        Environment.SetEnvironmentVariable("VEDASTRO_SVC_URL", "http://127.0.0.1:0");
        var paths = ServicePaths.Resolve();
        XmlDir = paths.XmlDir;
        DatasetDir = paths.DatasetDir;
        Predicates = new PredicateRegistry();
        Catalog = new RuleCatalog(XmlDir, Predicates);
        Store = new DatasetStore(DatasetDir, Path.Combine(Path.GetTempPath(), "vedastro-svc-tests-" + Environment.ProcessId));
    }

    public void Dispose() => Store.Dispose();
}

public sealed class CatalogTests : IClassFixture<Fixture>
{
    private readonly Fixture _f;
    public CatalogTests(Fixture f) => _f = f;

    private static int ParsedCount(string path) => XDocument.Load(path).Root!.Elements("Event").Count();

    [Fact]
    public void Proved_counts_equal_parsed_xml_counts()
    {
        var eventProved = ParsedCount(Path.Combine(_f.XmlDir, "EventDataList.xml"));
        var horoProved = ParsedCount(Path.Combine(_f.XmlDir, "HoroscopeDataList.xml"));
        Assert.Equal(1018, eventProved);
        Assert.Equal(490, horoProved); // 491 "<Event>" strings in the file, one is inside an XML comment (line 4995)
        Assert.Equal(eventProved, _f.Catalog.Count(RuleSet.Event, RuleStatus.Proved));
        Assert.Equal(horoProved, _f.Catalog.Count(RuleSet.Horoscope, RuleStatus.Proved));
        Assert.Equal(eventProved + horoProved, _f.Catalog.CountProved);
    }

    [Fact]
    public void Quarantined_are_only_the_not_proved_extras()
    {
        // EventDataList-not-proved.xml = 1028 (superset of the 1018 proved) => 10 quarantined
        // HoroscopeDataList-not-proved.xml = 123 elements / 118 unique names, none overlapping proved
        Assert.Equal(10, _f.Catalog.Count(RuleSet.Event, RuleStatus.Quarantined));
        Assert.Equal(118, _f.Catalog.Count(RuleSet.Horoscope, RuleStatus.Quarantined));
        Assert.Equal(128, _f.Catalog.CountQuarantined);
        var proved = new HashSet<string>(_f.Catalog.Query(null, RuleStatus.Proved).Select(r => r.Id));
        Assert.DoesNotContain(_f.Catalog.Query(null, RuleStatus.Quarantined), r => proved.Contains(r.Id));
    }

    [Fact]
    public void Every_proved_event_rule_has_a_compiled_predicate()
    {
        Assert.Equal(1018, _f.Catalog.CountWithPredicate(RuleSet.Event, RuleStatus.Proved));
        Assert.Equal(387, _f.Catalog.CountWithPredicate(RuleSet.Horoscope, RuleStatus.Proved)); // 103 newer rules have no public predicate
    }

    [Fact]
    public void Trimmed_library_references_no_cloud_or_http_assemblies()
    {
        var refs = typeof(VedAstro.Library.Calculate).Assembly.GetReferencedAssemblies().Select(a => a.Name ?? "").ToList();
        Assert.DoesNotContain(refs, n => n.StartsWith("Azure", StringComparison.Ordinal));
        Assert.DoesNotContain(refs, n => n.Contains("OpenAI", StringComparison.Ordinal));
        Assert.DoesNotContain(refs, n => n.Contains("Bing", StringComparison.Ordinal));
        Assert.DoesNotContain(refs, n => n == "System.Net.Http");
        Assert.DoesNotContain(refs, n => n == "System.Net.Requests");
    }

    [Fact]
    public void Secrets_resolve_only_from_env_and_default_to_empty()
    {
        Assert.Equal("", VedAstro.Library.Secrets.Get("AzureOpenAIAPIKey"));
        Environment.SetEnvironmentVariable("VEDASTRO_SECRET_UNITTESTKEY", "abc");
        try { Assert.Equal("abc", VedAstro.Library.Secrets.Get("UnitTestKey")); }
        finally { Environment.SetEnvironmentVariable("VEDASTRO_SECRET_UNITTESTKEY", null); }
        Assert.Equal("", VedAstro.Library.Secrets.Get(""));
    }
}

public sealed class MuhurtaTests : IClassFixture<Fixture>
{
    private readonly Fixture _f;
    public MuhurtaTests(Fixture f) => _f = f;

    private MuhurtaFindRequest Delhi(string activity, bool quarantined = false) =>
        new(activity, "2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z", 28.6139, 77.2090, 5.5, 60, quarantined);

    [Fact]
    public void GoodLunarDayForTravel_one_day_returns_windows_with_rule_trail()
    {
        var finder = new MuhurtaFinder(_f.Catalog, _f.Predicates, _f.Evidence);
        var res = finder.Find(Delhi("GoodLunarDayForTravel"));
        Assert.Equal(25, res.SlicesTotal);
        Assert.NotEmpty(res.Windows);
        Assert.Contains("GoodLunarDayForTravel", res.RulesEvaluated);
        Assert.Contains("BadLunarDayForTravel", res.RulesEvaluated); // sibling via tag "Travel"
        Assert.Empty(res.RulesQuarantinedEvaluated);
        Assert.True(res.Complete, string.Join("; ", res.RuleErrors.Select(kv => kv.Key + "=" + kv.Value)));
        Assert.Contains(res.Windows, w => w.PassedRules.Count > 0 || w.VetoedBy.Count > 0);
        Assert.All(res.Windows, w => Assert.All(w.PassedRules, t => Assert.Equal("Good", t.Nature)));
        Assert.All(res.Windows, w => Assert.All(w.VetoedBy, t => Assert.Equal("Bad", t.Nature)));
        Assert.Contains(res.Windows, w => w.ActivityRulePassed && w.PassedRules.Any(t => t.Id == "GoodLunarDayForTravel"));
        Assert.Equal(1, res.Windows[0].Rank);
        Assert.All(res.Windows, w => Assert.Empty(w.PassedRulesQuarantined));
    }

    [Fact]
    public void Quarantined_rules_excluded_by_default_and_tagged_when_included()
    {
        var finder = new MuhurtaFinder(_f.Catalog, _f.Predicates, _f.Evidence);
        var q = _f.Catalog.Query(RuleSet.Event, RuleStatus.Quarantined).First();
        var err = finder.Validate(Delhi(q.Id), out _, out _, out _);
        Assert.NotNull(err);
        Assert.Contains("quarantined", err);

        var res = finder.Find(Delhi(q.Id, quarantined: true));
        Assert.Contains(q.Id, res.RulesQuarantinedEvaluated.Concat(res.RulesUnevaluable));
        Assert.DoesNotContain(q.Id, res.RulesEvaluated);
        Assert.All(res.Windows, w => Assert.DoesNotContain(w.PassedRules.Concat(w.VetoedBy), t => t.Status == "quarantined"));
    }

    [Fact]
    public void Lost_pancha_pakshi_predicates_are_reported_unevaluable_not_guessed()
    {
        var finder = new MuhurtaFinder(_f.Catalog, _f.Predicates, _f.Evidence);
        var res = finder.Find(Delhi("BirdRuling", quarantined: true)); // the 5 Bird* + 5 Yama* rules are exactly the 10 quarantined-only event rules
        Assert.Contains("BirdRuling", res.RulesUnevaluable);
        Assert.DoesNotContain("BirdRuling", res.RulesEvaluated);
        Assert.DoesNotContain("BirdRuling", res.RulesQuarantinedEvaluated);
    }

    [Fact]
    public void Validation_rejects_bad_input()
    {
        var finder = new MuhurtaFinder(_f.Catalog, _f.Predicates, _f.Evidence);
        Assert.NotNull(finder.Validate(Delhi("NoSuchRule"), out _, out _, out _));
        Assert.NotNull(finder.Validate(Delhi("GoodLunarDayForTravel") with { ToUtc = "2026-09-30T00:00:00Z" }, out _, out _, out _));
        Assert.NotNull(finder.Validate(Delhi("GoodLunarDayForTravel") with { StepMinutes = 0 }, out _, out _, out _));
        Assert.NotNull(finder.Validate(Delhi("GoodLunarDayForTravel") with { Lat = 95 }, out _, out _, out _));
        Assert.NotNull(finder.Validate(Delhi("GoodLunarDayForTravel") with { ToUtc = "2030-01-01T00:00:00Z", StepMinutes = 1 }, out _, out _, out _));
        Assert.NotNull(finder.Validate(Delhi("SunAshtakavargaYoga2"), out _, out _, out _)); // horoscope rule, not event
    }
}

public sealed class ValidateTests : IClassFixture<Fixture>
{
    private readonly Fixture _f;
    public ValidateTests(Fixture f) => _f = f;

    [Fact]
    public void Wilson_interval_known_values()
    {
        var (lo, hi) = RuleValidator.Wilson95(0, 0);
        Assert.Equal((0, 0), (lo, hi));
        (lo, hi) = RuleValidator.Wilson95(50, 100);
        Assert.InRange(lo, 0.4038, 0.4040); // textbook: 0.4038..0.5962
        Assert.InRange(hi, 0.5960, 0.5962);
        (lo, hi) = RuleValidator.Wilson95(100, 100);
        Assert.InRange(lo, 0.963, 0.964);
        Assert.Equal(1, hi);
    }

    [Fact]
    public void Validate_one_horoscope_rule_on_marriage_dataset_50_rows()
    {
        var v = new RuleValidator(_f.Catalog, _f.Predicates, _f.Store, _f.Evidence);
        var rule = _f.Catalog.Query(RuleSet.Horoscope, RuleStatus.Proved).First(r => r.HasPredicate && r.Id.Contains("Lagna", StringComparison.Ordinal));
        var res = v.Run(new RuleValidateRequest(rule.Id, "marriage", "any_dissolution", 50));
        Assert.Equal(rule.Id, res.RuleId);
        Assert.InRange(res.N, 1, 50);
        Assert.InRange(res.Fired, 0, res.N);
        Assert.InRange(res.Hits, 0, res.Fired);
        Assert.InRange(res.HitRate, 0, 1);
        Assert.InRange(res.BaseRate, 0, 1);
        Assert.InRange(res.Ci95.Lo, 0, res.Ci95.Hi);
        Assert.InRange(res.Ci95.Hi, res.Ci95.Lo, 1);
        Assert.Equal("KEEP_UNPROVED", res.Verdict); // fired < 200 can never PROMOTE
        Assert.True(res.RowErrors <= 5, $"row errors {res.RowErrors}");
        Assert.NotNull(res.Evidence.DatasetSha256);
        Assert.Equal(15807, _f.Store.PersonRows);
    }

    [Fact]
    public void Validate_rejects_unknown_rule_dataset_and_column()
    {
        var v = new RuleValidator(_f.Catalog, _f.Predicates, _f.Store, _f.Evidence);
        Assert.NotNull(v.Validate(new RuleValidateRequest("Nope", "marriage", "married"), out _));
        Assert.NotNull(v.Validate(new RuleValidateRequest("SunAshtakavargaYoga2", "marriage", "married"), out _)); // predicate_missing
        Assert.NotNull(v.Validate(new RuleValidateRequest("GoodLunarDayForTravel", "marriage", "married"), out _)); // event rule
        Assert.NotNull(v.Validate(new RuleValidateRequest("MoonInLagna", "nope", "married"), out _));
        Assert.NotNull(v.Validate(new RuleValidateRequest("MoonInLagna", "marriage", "drop table"), out _));
        Assert.NotNull(v.Validate(new RuleValidateRequest("MoonInLagna", "marriage", "married", 0), out _));
    }
}

public sealed class HttpTests : IClassFixture<WebApplicationFactory<Program>>
{
    private readonly WebApplicationFactory<Program> _factory;
    public HttpTests(WebApplicationFactory<Program> factory)
    {
        Environment.SetEnvironmentVariable("VEDASTRO_SVC_URL", "http://127.0.0.1:0");
        _factory = factory;
    }

    [Fact]
    public async Task Health_reports_counts()
    {
        var client = _factory.CreateClient();
        var json = await client.GetFromJsonAsync<JsonElement>("/v1/health");
        Assert.True(json.GetProperty("ok").GetBoolean());
        Assert.Equal(1508, json.GetProperty("rules_proved").GetInt32());
        Assert.Equal(128, json.GetProperty("rules_quarantined").GetInt32());
        Assert.Equal(15807, json.GetProperty("dataset_rows").GetInt32());
        Assert.Equal("LAHIRI", json.GetProperty("ayanamsa").GetString());
        Assert.True(json.GetProperty("true_nodes").GetBoolean());
    }

    [Fact]
    public async Task Rules_endpoint_filters()
    {
        var client = _factory.CreateClient();
        var q = await client.GetFromJsonAsync<JsonElement>("/v1/rules?set=event&status=quarantined");
        Assert.Equal(10, q.GetProperty("count").GetInt32());
        var first = q.GetProperty("rules")[0];
        foreach (var k in new[] { "id", "name", "description", "tags" }) { Assert.True(first.TryGetProperty(k, out _), k); }
        var bad = await client.GetAsync("/v1/rules?set=nope");
        Assert.Equal(System.Net.HttpStatusCode.BadRequest, bad.StatusCode);
    }

    [Fact]
    public async Task Muhurta_find_http_roundtrip()
    {
        var client = _factory.CreateClient();
        var res = await client.PostAsJsonAsync("/v1/muhurta/find", new { activity = "GoodLunarDayForTravel", from_utc = "2026-10-01T00:00:00Z", to_utc = "2026-10-01T12:00:00Z", lat = 28.6139, lon = 77.2090, tz_offset_hours = 5.5, step_minutes = 60 });
        Assert.Equal(System.Net.HttpStatusCode.OK, res.StatusCode);
        var json = await res.Content.ReadFromJsonAsync<JsonElement>();
        Assert.True(json.GetProperty("windows").GetArrayLength() >= 1);
        Assert.True(json.GetProperty("windows")[0].TryGetProperty("passed_rules", out _));
        Assert.True(json.GetProperty("windows")[0].TryGetProperty("vetoed_by", out _));
        var bad = await client.PostAsJsonAsync("/v1/muhurta/find", new { activity = "GoodLunarDayForTravel" });
        Assert.Equal(System.Net.HttpStatusCode.BadRequest, bad.StatusCode);
    }
}
