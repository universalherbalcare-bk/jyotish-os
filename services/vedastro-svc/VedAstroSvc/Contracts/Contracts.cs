using System.Text.Json.Serialization;

namespace VedAstroSvc.Contracts;

/// <summary>CONTRACT.md shared input type. <c>utc</c> is authoritative; <c>tz_offset_hours</c> only positions the local day.</summary>
public sealed record BirthInput(
    [property: JsonPropertyName("utc")] string Utc,
    [property: JsonPropertyName("lat")] double Lat,
    [property: JsonPropertyName("lon")] double Lon,
    [property: JsonPropertyName("tz_offset_hours")] double TzOffsetHours);

public sealed record MuhurtaFindRequest(
    [property: JsonPropertyName("activity")] string Activity,
    [property: JsonPropertyName("from_utc")] string FromUtc,
    [property: JsonPropertyName("to_utc")] string ToUtc,
    [property: JsonPropertyName("lat")] double Lat,
    [property: JsonPropertyName("lon")] double Lon,
    [property: JsonPropertyName("tz_offset_hours")] double TzOffsetHours,
    [property: JsonPropertyName("step_minutes")] int StepMinutes = 60,
    [property: JsonPropertyName("include_quarantined")] bool IncludeQuarantined = false,
    [property: JsonPropertyName("birth")] BirthInput? Birth = null);

public sealed record RuleTrailItem(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("nature")] string Nature,
    [property: JsonPropertyName("status")] string Status,
    [property: JsonPropertyName("slices")] int Slices);

public sealed record MuhurtaWindow(
    [property: JsonPropertyName("rank")] int Rank,
    [property: JsonPropertyName("start_utc")] string StartUtc,
    [property: JsonPropertyName("end_utc")] string EndUtc,
    [property: JsonPropertyName("start_local")] string StartLocal,
    [property: JsonPropertyName("end_local")] string EndLocal,
    [property: JsonPropertyName("duration_minutes")] int DurationMinutes,
    [property: JsonPropertyName("slices")] int Slices,
    [property: JsonPropertyName("score")] double Score,
    [property: JsonPropertyName("activity_rule_passed")] bool ActivityRulePassed,
    [property: JsonPropertyName("passed_rules")] IReadOnlyList<RuleTrailItem> PassedRules,
    [property: JsonPropertyName("vetoed_by")] IReadOnlyList<RuleTrailItem> VetoedBy,
    [property: JsonPropertyName("fired_neutral")] IReadOnlyList<RuleTrailItem> FiredNeutral,
    [property: JsonPropertyName("passed_rules_quarantined")] IReadOnlyList<RuleTrailItem> PassedRulesQuarantined,
    [property: JsonPropertyName("vetoed_by_quarantined")] IReadOnlyList<RuleTrailItem> VetoedByQuarantined,
    [property: JsonPropertyName("fired_neutral_quarantined")] IReadOnlyList<RuleTrailItem> FiredNeutralQuarantined);

public sealed record MuhurtaFindResponse(
    [property: JsonPropertyName("activity")] string Activity,
    [property: JsonPropertyName("activity_tags")] IReadOnlyList<string> ActivityTags,
    [property: JsonPropertyName("rules_evaluated")] IReadOnlyList<string> RulesEvaluated,
    [property: JsonPropertyName("rules_quarantined_evaluated")] IReadOnlyList<string> RulesQuarantinedEvaluated,
    [property: JsonPropertyName("rules_unevaluable")] IReadOnlyList<string> RulesUnevaluable,
    [property: JsonPropertyName("rule_errors")] IReadOnlyDictionary<string, string> RuleErrors,
    [property: JsonPropertyName("slices_total")] int SlicesTotal,
    [property: JsonPropertyName("step_minutes")] int StepMinutes,
    [property: JsonPropertyName("birth_supplied")] bool BirthSupplied,
    [property: JsonPropertyName("complete")] bool Complete,
    [property: JsonPropertyName("windows")] IReadOnlyList<MuhurtaWindow> Windows,
    [property: JsonPropertyName("evidence")] Evidence Evidence);

public sealed record RuleValidateRequest(
    [property: JsonPropertyName("rule_id")] string RuleId,
    [property: JsonPropertyName("dataset")] string Dataset,
    [property: JsonPropertyName("outcome_column")] string OutcomeColumn,
    [property: JsonPropertyName("max_rows")] int MaxRows = 300,
    [property: JsonPropertyName("offset")] int Offset = 0);

public sealed record Ci95(
    [property: JsonPropertyName("lo")] double Lo,
    [property: JsonPropertyName("hi")] double Hi);

public sealed record RuleValidateResponse(
    [property: JsonPropertyName("rule_id")] string RuleId,
    [property: JsonPropertyName("rule_status")] string RuleStatus,
    [property: JsonPropertyName("dataset")] string Dataset,
    [property: JsonPropertyName("outcome_column")] string OutcomeColumn,
    [property: JsonPropertyName("n")] int N,
    [property: JsonPropertyName("fired")] int Fired,
    [property: JsonPropertyName("hits")] int Hits,
    [property: JsonPropertyName("hit_rate")] double HitRate,
    [property: JsonPropertyName("base_rate")] double BaseRate,
    [property: JsonPropertyName("base_rate_full_dataset")] double BaseRateFullDataset,
    [property: JsonPropertyName("ci95")] Ci95 Ci95,
    [property: JsonPropertyName("verdict")] string Verdict,
    [property: JsonPropertyName("verdict_reason")] string VerdictReason,
    [property: JsonPropertyName("row_errors")] int RowErrors,
    [property: JsonPropertyName("elapsed_ms")] long ElapsedMs,
    [property: JsonPropertyName("evidence")] Evidence Evidence);

public sealed record Evidence(
    [property: JsonPropertyName("engine")] string Engine,
    [property: JsonPropertyName("kernel")] string Kernel,
    [property: JsonPropertyName("ayanamsa")] string Ayanamsa,
    [property: JsonPropertyName("true_nodes")] bool TrueNodes,
    [property: JsonPropertyName("positions_role")] string PositionsRole,
    [property: JsonPropertyName("dataset_sha256")] string? DatasetSha256 = null);

public sealed record ErrorResponse(
    [property: JsonPropertyName("error")] string Error,
    [property: JsonPropertyName("detail")] string Detail);
