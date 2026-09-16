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
    public PromotionLog Promotions { get; }
    public Evidence Evidence { get; } = new("test", "test", "LAHIRI", true, "predicates-only");
    public string XmlDir { get; }
    public string DatasetDir { get; }
    public string TempDir { get; }

    public Fixture()
    {
        Environment.SetEnvironmentVariable("VEDASTRO_SVC_URL", "http://127.0.0.1:0");
        var paths = ServicePaths.Resolve();
        XmlDir = paths.XmlDir;
        DatasetDir = paths.DatasetDir;
        Predicates = new PredicateRegistry();
        Catalog = new RuleCatalog(XmlDir, Predicates);
        TempDir = Path.Combine(Path.GetTempPath(), "vedastro-svc-tests-" + Environment.ProcessId);
        Store = new DatasetStore(DatasetDir, TempDir);
        Promotions = new PromotionLog(Path.Combine(TempDir, "fixture-promotions.jsonl"));
    }

    public PromotionLog FreshLog(string name)
    {
        var path = Path.Combine(TempDir, name + "-" + Guid.NewGuid().ToString("N") + ".jsonl");
        return new PromotionLog(path);
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
        var v = new RuleValidator(_f.Catalog, _f.Predicates, _f.Store, _f.Evidence, _f.Promotions);
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
        Assert.Equal("NOT_PROMOTED", res.ValidationStatus);
        Assert.Equal("NOT_PROMOTED", res.PromotionStatus);
        Assert.Equal(64, res.PromotionEntryHash.Length);
    }

    [Fact]
    public void Validate_rejects_unknown_rule_dataset_and_column()
    {
        var v = new RuleValidator(_f.Catalog, _f.Predicates, _f.Store, _f.Evidence, _f.Promotions);
        Assert.NotNull(v.Validate(new RuleValidateRequest("Nope", "marriage", "married"), out _));
        Assert.NotNull(v.Validate(new RuleValidateRequest("SunAshtakavargaYoga2", "marriage", "married"), out _)); // predicate_missing
        Assert.NotNull(v.Validate(new RuleValidateRequest("GoodLunarDayForTravel", "marriage", "married"), out _)); // event rule
        Assert.NotNull(v.Validate(new RuleValidateRequest("SaturnIn7thNotLagnaLord", "nope", "married"), out _));
        Assert.NotNull(v.Validate(new RuleValidateRequest("SaturnIn7thNotLagnaLord", "marriage", "drop table"), out _));
        Assert.NotNull(v.Validate(new RuleValidateRequest("SaturnIn7thNotLagnaLord", "marriage", "married", 0), out _));
    }
}

public sealed class PromotionTests : IClassFixture<Fixture>
{
    private readonly Fixture _f;
    public PromotionTests(Fixture f) => _f = f;

    private static PromotionEntry Draft(string ruleId, string verdict, int fired = 250, int hits = 200) =>
        new("2026-09-17T00:00:00.0000000Z", ruleId, "marriage", new string('a', 64), "married", 1000, fired, hits, Math.Round(hits / (double)fired, 6), 0.4, new[] { 0.1, 0.2 }, verdict, PromotionLog.GenesisHash);

    [Fact]
    public void Chain_grows_by_one_per_validate_and_verifies()
    {
        var log = _f.FreshLog("grow");
        var v = new RuleValidator(_f.Catalog, _f.Predicates, _f.Store, _f.Evidence, log);
        var rule = _f.Catalog.Query(RuleSet.Horoscope, RuleStatus.Proved).First(r => r.HasPredicate && r.Id.Contains("Lagna", StringComparison.Ordinal));
        Assert.Empty(log.ReadAll());
        Assert.True(log.Verify().Ok);
        Assert.Equal(0, log.Verify().Entries);

        var r1 = v.Run(new RuleValidateRequest(rule.Id, "marriage", "any_dissolution", 20));
        var after1 = log.ReadAll();
        Assert.Single(after1);
        Assert.Equal(PromotionLog.GenesisHash, after1[0].PrevHash);
        Assert.Equal(r1.PromotionEntryHash, after1[0].EntryHash);
        Assert.Equal(rule.Id, after1[0].RuleId);
        Assert.Equal(r1.N, after1[0].N);
        Assert.Equal(r1.Fired, after1[0].Fired);
        Assert.Equal(r1.Hits, after1[0].Hits);
        Assert.Equal(r1.Ci95.Lo, after1[0].Ci95[0]);
        Assert.Equal(r1.Ci95.Hi, after1[0].Ci95[1]);
        Assert.Equal(r1.Evidence.DatasetSha256, after1[0].DatasetSha256);
        Assert.Equal(r1.Verdict, after1[0].Verdict);

        var r2 = v.Run(new RuleValidateRequest(rule.Id, "marriage", "any_dissolution", 20, 20));
        var after2 = log.ReadAll();
        Assert.Equal(2, after2.Count);
        Assert.Equal(after1[0].EntryHash, after2[1].PrevHash);
        Assert.Equal(r2.PromotionEntryHash, after2[1].EntryHash);
        Assert.NotEqual(after2[0].EntryHash, after2[1].EntryHash);

        var ver = log.Verify();
        Assert.True(ver.Ok, ver.Detail);
        Assert.Equal(2, ver.Entries);
        Assert.Null(ver.FirstBadIndex);
        Assert.Equal(after2[1].EntryHash, ver.HeadHash);

        // entry_hash is exactly sha256(prev_hash + canonical JSON without entry_hash)
        var recomputed = PromotionLog.ComputeHash(after2[1].PrevHash, after2[1] with { EntryHash = null });
        Assert.Equal(after2[1].EntryHash, recomputed);
        Assert.DoesNotContain("entry_hash", PromotionLog.CanonicalJson(after2[1]));
        Assert.StartsWith("{\"ts\":", PromotionLog.CanonicalJson(after2[1]));

        // each validate appended exactly one line
        Assert.Equal(2, File.ReadAllLines(log.Path).Length);
    }

    [Fact]
    public void Verify_reports_first_bad_index_for_edited_and_deleted_lines()
    {
        var log = _f.FreshLog("tamper");
        log.Append(Draft("RuleA", "KEEP_UNPROVED"));
        log.Append(Draft("RuleB", "KEEP_UNPROVED"));
        log.Append(Draft("RuleC", "KEEP_UNPROVED"));
        Assert.True(log.Verify().Ok);
        var lines = File.ReadAllLines(log.Path);
        Assert.Equal(3, lines.Length);

        // edit one field on line 1 in a temp copy → chain breaks at index 1
        var edited = Path.Combine(_f.TempDir, "tamper-edited-" + Guid.NewGuid().ToString("N") + ".jsonl");
        var l1 = lines[1].Replace("\"hits\":200", "\"hits\":999", StringComparison.Ordinal);
        Assert.NotEqual(lines[1], l1);
        File.WriteAllLines(edited, new[] { lines[0], l1, lines[2] });
        var v = new PromotionLog(edited).Verify();
        Assert.False(v.Ok);
        Assert.Equal(1, v.FirstBadIndex);
        Assert.Equal(3, File.ReadAllLines(edited).Length);

        // delete line 1 → line 2's prev_hash no longer matches → index 1
        var deleted = Path.Combine(_f.TempDir, "tamper-deleted-" + Guid.NewGuid().ToString("N") + ".jsonl");
        File.WriteAllLines(deleted, new[] { lines[0], lines[2] });
        v = new PromotionLog(deleted).Verify();
        Assert.False(v.Ok);
        Assert.Equal(1, v.FirstBadIndex);

        // garbage line → index 0
        var garbage = Path.Combine(_f.TempDir, "tamper-garbage-" + Guid.NewGuid().ToString("N") + ".jsonl");
        File.WriteAllLines(garbage, new[] { "not json", lines[1] });
        v = new PromotionLog(garbage).Verify();
        Assert.False(v.Ok);
        Assert.Equal(0, v.FirstBadIndex);

        // a broken chain refuses further appends (fail closed) and the file is untouched
        var before = new FileInfo(edited).Length;
        Assert.Throws<PromotionLogException>(() => new PromotionLog(edited).Append(Draft("RuleD", "KEEP_UNPROVED")));
        Assert.Equal(before, new FileInfo(edited).Length);

        // the untampered original still verifies
        Assert.True(log.Verify().Ok);
    }

    [Fact]
    public void Promote_requires_fired_200_and_ci_lo_above_base_rate_and_only_then_status_flips()
    {
        // gate: fired < 200 never promotes, even with a perfect hit rate
        var d = RuleValidator.Decide(n: 1000, fired: 199, hits: 199, outcomeTrue: 100);
        Assert.Equal("KEEP_UNPROVED", d.Verdict);
        // gate: fired >= 200 but ci95.lo <= base_rate keeps unproved
        d = RuleValidator.Decide(n: 1000, fired: 200, hits: 60, outcomeTrue: 300);
        Assert.Equal("KEEP_UNPROVED", d.Verdict);
        Assert.True(d.Lo <= d.BaseRate);
        // gate: fired >= 200 and ci95.lo > base_rate promotes
        d = RuleValidator.Decide(n: 1000, fired: 200, hits: 120, outcomeTrue: 300);
        Assert.Equal("PROMOTE", d.Verdict);
        Assert.True(d.Lo > d.BaseRate);
        Assert.Equal(0.6, d.HitRate, 9);
        Assert.Equal(0.3, d.BaseRate, 9);
        Assert.Throws<ArgumentOutOfRangeException>(() => RuleValidator.Decide(10, 20, 0, 0));

        // status is derived from the latest log row for the rule, never from memory
        var log = _f.FreshLog("status");
        Assert.Equal("NEVER_VALIDATED", log.StatusOf("RuleX"));
        log.Append(Draft("RuleX", "KEEP_UNPROVED"));
        Assert.Equal("NOT_PROMOTED", log.StatusOf("RuleX"));
        log.Append(Draft("RuleX", "PROMOTE"));
        Assert.Equal("PROMOTED", log.StatusOf("RuleX"));
        Assert.Equal("NEVER_VALIDATED", log.StatusOf("RuleY"));
        log.Append(Draft("RuleX", "KEEP_UNPROVED")); // a later KEEP_UNPROVED demotes: latest row wins
        Assert.Equal("NOT_PROMOTED", log.StatusOf("RuleX"));
        var byRule = new PromotionLog(log.Path).LatestStatusByRule(); // fresh instance, same file
        Assert.Equal("NOT_PROMOTED", byRule["RuleX"]);

        // promotion never changes quarantine: a quarantined rule with PROMOTE stays out of muhurta
        Assert.Equal("PROMOTED_STILL_QUARANTINED", RuleValidator.ValidationStatus("PROMOTED", RuleStatus.Quarantined));
        Assert.Equal("PROMOTED", RuleValidator.ValidationStatus("PROMOTED", RuleStatus.Proved));
        Assert.Equal("NOT_PROMOTED", RuleValidator.ValidationStatus("NOT_PROMOTED", RuleStatus.Proved));
        Assert.Equal("NOT_PROMOTED", RuleValidator.ValidationStatus("NEVER_VALIDATED", RuleStatus.Quarantined));
        Assert.Contains("quarantined rules stay excluded", RuleValidator.PromotionScope);
    }

    [Fact]
    public void Log_is_append_only_and_exposes_no_mutating_api()
    {
        var log = _f.FreshLog("appendonly");
        long last = 0;
        for (var i = 0; i < 5; i++)
        {
            log.Append(Draft("RuleZ", "KEEP_UNPROVED"));
            var len = new FileInfo(log.Path).Length;
            Assert.True(len > last, "file must only grow");
            last = len;
        }
        var publicMethods = typeof(PromotionLog).GetMethods(System.Reflection.BindingFlags.Public | System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.DeclaredOnly)
            .Select(m => m.Name).Where(n => !n.StartsWith("get_", StringComparison.Ordinal)).OrderBy(n => n).ToArray();
        Assert.Equal(new[] { "Append", "LatestStatusByRule", "ReadAll", "StatusOf", "Verify" }, publicMethods);
        Assert.DoesNotContain(publicMethods, n => n.Contains("Truncate", StringComparison.OrdinalIgnoreCase) || n.Contains("Delete", StringComparison.OrdinalIgnoreCase) || n.Contains("Clear", StringComparison.OrdinalIgnoreCase) || n.Contains("Rewrite", StringComparison.OrdinalIgnoreCase));
    }
}

public sealed class HttpTests : IClassFixture<WebApplicationFactory<Program>>
{
    private readonly WebApplicationFactory<Program> _factory;
    private static readonly string PromotionLogPath = Path.Combine(Path.GetTempPath(), "vedastro-svc-tests-" + Environment.ProcessId, "http-promotions.jsonl");

    public HttpTests(WebApplicationFactory<Program> factory)
    {
        Environment.SetEnvironmentVariable("VEDASTRO_SVC_URL", "http://127.0.0.1:0");
        Environment.SetEnvironmentVariable("VEDASTRO_PROMOTION_LOG", PromotionLogPath);
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
        Assert.True(json.GetProperty("promotion_log").GetProperty("chain_ok").GetBoolean());
        Assert.Equal(PromotionLogPath, json.GetProperty("promotion_log").GetProperty("path").GetString());
    }

    [Fact]
    public async Task Validate_appends_to_promotion_log_and_rules_report_promotion_status()
    {
        var client = _factory.CreateClient();
        var v0 = await client.GetFromJsonAsync<JsonElement>("/v1/rule/promotions/verify");
        Assert.True(v0.GetProperty("ok").GetBoolean());
        var before = v0.GetProperty("entries").GetInt32();

        var body = new { rule_id = "SaturnIn7thNotLagnaLord", dataset = "marriage", outcome_column = "married", max_rows = 10 };
        var r1 = await client.PostAsJsonAsync("/v1/rule/validate", body);
        Assert.Equal(System.Net.HttpStatusCode.OK, r1.StatusCode);
        var j1 = await r1.Content.ReadFromJsonAsync<JsonElement>();
        Assert.Equal("KEEP_UNPROVED", j1.GetProperty("verdict").GetString());
        Assert.Equal("NOT_PROMOTED", j1.GetProperty("validation_status").GetString());
        Assert.Equal("NOT_PROMOTED", j1.GetProperty("promotion_status").GetString());
        Assert.Equal(64, j1.GetProperty("promotion_entry_hash").GetString()!.Length);
        var r2 = await client.PostAsJsonAsync("/v1/rule/validate", new { rule_id = "SaturnIn7thNotLagnaLord", dataset = "marriage", outcome_column = "married", max_rows = 10, offset = 10 });
        Assert.Equal(System.Net.HttpStatusCode.OK, r2.StatusCode);

        var ver = await client.GetFromJsonAsync<JsonElement>("/v1/rule/promotions/verify");
        Assert.True(ver.GetProperty("ok").GetBoolean());
        Assert.Equal(before + 2, ver.GetProperty("entries").GetInt32());
        Assert.Equal(JsonValueKind.Null, ver.GetProperty("first_bad_index").ValueKind);

        var all = await client.GetFromJsonAsync<JsonElement>("/v1/rule/promotions?rule_id=SaturnIn7thNotLagnaLord");
        Assert.True(all.GetProperty("count").GetInt32() >= 2);
        var last = all.GetProperty("entries")[all.GetProperty("count").GetInt32() - 1];
        foreach (var k in new[] { "ts", "rule_id", "dataset", "dataset_sha256", "outcome_column", "n", "fired", "hits", "hit_rate", "base_rate", "ci95", "verdict", "prev_hash", "entry_hash" }) { Assert.True(last.TryGetProperty(k, out _), k); }
        Assert.Equal(2, last.GetProperty("ci95").GetArrayLength());

        var rules = await client.GetFromJsonAsync<JsonElement>("/v1/rules?set=horoscope&status=proved");
        var moon = rules.GetProperty("rules").EnumerateArray().First(r => r.GetProperty("id").GetString() == "SaturnIn7thNotLagnaLord");
        Assert.Equal("NOT_PROMOTED", moon.GetProperty("promotion_status").GetString());
        Assert.Contains(rules.GetProperty("rules").EnumerateArray(), r => r.GetProperty("promotion_status").GetString() == "NEVER_VALIDATED");

        // no write/truncate route exists for the log
        Assert.Equal(System.Net.HttpStatusCode.MethodNotAllowed, (await client.DeleteAsync("/v1/rule/promotions")).StatusCode);
        Assert.Equal(System.Net.HttpStatusCode.MethodNotAllowed, (await client.PostAsync("/v1/rule/promotions", null)).StatusCode);
        Assert.Equal(System.Net.HttpStatusCode.MethodNotAllowed, (await client.PutAsync("/v1/rule/promotions", null)).StatusCode);
        Assert.Equal(System.Net.HttpStatusCode.MethodNotAllowed, (await client.DeleteAsync("/v1/rule/promotions/verify")).StatusCode);
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
