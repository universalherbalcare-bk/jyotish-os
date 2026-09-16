using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace VedAstroSvc.Validate;

/// <summary>Raised when the promotion log cannot be appended to or is found broken. The caller must fail closed (no verdict without a log row).</summary>
public sealed class PromotionLogException : Exception
{
    public PromotionLogException(string message, Exception? inner = null) : base(message, inner) { }
}

/// <summary>
/// One row of the append-only promotion log (blueprint §6 "Data integrity of rule promotions").
/// Property order is the canonical order: <c>entry_hash = sha256(prev_hash + canonical JSON of the row with entry_hash omitted)</c>,
/// where canonical JSON is System.Text.Json compact output (no whitespace, default encoder, properties in the order declared here).
/// </summary>
public sealed record PromotionEntry(
    [property: JsonPropertyName("ts")] string Ts,
    [property: JsonPropertyName("rule_id")] string RuleId,
    [property: JsonPropertyName("dataset")] string Dataset,
    [property: JsonPropertyName("dataset_sha256")] string DatasetSha256,
    [property: JsonPropertyName("outcome_column")] string OutcomeColumn,
    [property: JsonPropertyName("n")] int N,
    [property: JsonPropertyName("fired")] int Fired,
    [property: JsonPropertyName("hits")] int Hits,
    [property: JsonPropertyName("hit_rate")] double HitRate,
    [property: JsonPropertyName("base_rate")] double BaseRate,
    [property: JsonPropertyName("ci95")] double[] Ci95,
    [property: JsonPropertyName("verdict")] string Verdict,
    [property: JsonPropertyName("prev_hash")] string PrevHash,
    [property: JsonPropertyName("entry_hash"), JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] string? EntryHash = null);

public sealed record PromotionVerifyResult(
    [property: JsonPropertyName("ok")] bool Ok,
    [property: JsonPropertyName("entries")] int Entries,
    [property: JsonPropertyName("first_bad_index")] int? FirstBadIndex,
    [property: JsonPropertyName("head_hash")] string HeadHash,
    [property: JsonPropertyName("path")] string Path,
    [property: JsonPropertyName("detail")] string? Detail);

/// <summary>
/// Hash-chained, append-only JSONL log of every <c>rule.validate</c> verdict. The file is only ever opened with
/// <see cref="FileMode.Append"/> (the OS refuses seeks before the original end) and every append is fsync'd.
/// There is deliberately no API that rewrites, truncates or deletes the log. The chain is re-derived from the
/// file on every read — nothing about promotion state is held in memory. Deleting or editing an interior line
/// breaks the chain (the next row's <c>prev_hash</c> no longer matches); truncating the tail is only detectable
/// against an external anchor, which is why <see cref="Verify"/> reports <c>head_hash</c> for that purpose.
/// </summary>
public sealed class PromotionLog
{
    public const string GenesisHash = "0000000000000000000000000000000000000000000000000000000000000000";
    public const string PromotedStatus = "PROMOTED";
    public const string NotPromotedStatus = "NOT_PROMOTED";
    public const string NeverValidatedStatus = "NEVER_VALIDATED";

    private static readonly JsonSerializerOptions Canonical = new()
    {
        WriteIndented = false,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        NumberHandling = JsonNumberHandling.Strict,
    };

    private readonly object _gate = new();

    public string Path { get; }

    public PromotionLog(string path)
    {
        if (string.IsNullOrWhiteSpace(path)) { throw new ArgumentException("promotion log path is empty", nameof(path)); }
        Path = System.IO.Path.GetFullPath(path);
        var dir = System.IO.Path.GetDirectoryName(Path);
        if (!string.IsNullOrEmpty(dir)) { Directory.CreateDirectory(dir); }
    }

    /// <summary>Canonical JSON of a row with <c>entry_hash</c> omitted.</summary>
    public static string CanonicalJson(PromotionEntry e) => JsonSerializer.Serialize(e with { EntryHash = null }, Canonical);

    /// <summary><c>sha256(prev_hash + canonical JSON)</c>, lowercase hex.</summary>
    public static string ComputeHash(string prevHash, PromotionEntry e) =>
        Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(prevHash + CanonicalJson(e))));

    /// <summary>
    /// Appends one row and returns it with <c>prev_hash</c>/<c>entry_hash</c> filled in. The existing chain is verified
    /// first; a broken chain refuses the append (fail closed) rather than extending a tampered log.
    /// </summary>
    public PromotionEntry Append(PromotionEntry draft)
    {
        if (string.IsNullOrWhiteSpace(draft.RuleId)) { throw new PromotionLogException("rule_id is required"); }
        if (draft.Ci95 is null || draft.Ci95.Length != 2) { throw new PromotionLogException("ci95 must be [lo,hi]"); }
        lock (_gate)
        {
            var state = VerifyLocked();
            if (!state.Ok)
            {
                throw new PromotionLogException($"promotion log chain broken at index {state.FirstBadIndex}: {state.Detail}; refusing to append");
            }
            var entry = draft with { PrevHash = state.HeadHash, EntryHash = null };
            entry = entry with { EntryHash = ComputeHash(state.HeadHash, entry) };
            var line = JsonSerializer.Serialize(entry, Canonical) + "\n";
            try
            {
                using var fs = new FileStream(Path, FileMode.Append, FileAccess.Write, FileShare.Read, bufferSize: 4096, FileOptions.WriteThrough);
                var bytes = Encoding.UTF8.GetBytes(line);
                fs.Write(bytes, 0, bytes.Length);
                fs.Flush(flushToDisk: true);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                throw new PromotionLogException($"cannot append to promotion log {Path}: {ex.Message}", ex);
            }
            return entry;
        }
    }

    /// <summary>Every row in file order (rows that fail to parse are skipped here; <see cref="Verify"/> reports them).</summary>
    public IReadOnlyList<PromotionEntry> ReadAll(string? ruleId = null)
    {
        lock (_gate)
        {
            var list = new List<PromotionEntry>();
            foreach (var line in ReadLinesLocked())
            {
                var e = TryParse(line);
                if (e is null) { continue; }
                if (ruleId is not null && !string.Equals(e.RuleId, ruleId, StringComparison.Ordinal)) { continue; }
                list.Add(e);
            }
            return list;
        }
    }

    /// <summary>Re-hashes the whole chain from disk.</summary>
    public PromotionVerifyResult Verify()
    {
        lock (_gate) { return VerifyLocked(); }
    }

    /// <summary>Latest verdict per rule, derived from the file: PROMOTED iff the newest row for that rule says PROMOTE.</summary>
    public IReadOnlyDictionary<string, string> LatestStatusByRule()
    {
        var map = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var e in ReadAll())
        {
            map[e.RuleId] = e.Verdict == "PROMOTE" ? PromotedStatus : NotPromotedStatus;
        }
        return map;
    }

    public string StatusOf(string ruleId) => LatestStatusByRule().TryGetValue(ruleId, out var s) ? s : NeverValidatedStatus;

    private PromotionVerifyResult VerifyLocked()
    {
        var prev = GenesisHash;
        var i = 0;
        foreach (var line in ReadLinesLocked())
        {
            var e = TryParse(line);
            if (e is null)
            {
                return new PromotionVerifyResult(false, i, i, prev, Path, "line is not a valid promotion entry");
            }
            if (!string.Equals(e.PrevHash, prev, StringComparison.Ordinal))
            {
                return new PromotionVerifyResult(false, i, i, prev, Path, "prev_hash does not match the previous entry_hash (row edited, inserted or deleted)");
            }
            var expected = ComputeHash(prev, e);
            if (!string.Equals(e.EntryHash, expected, StringComparison.Ordinal))
            {
                return new PromotionVerifyResult(false, i, i, prev, Path, "entry_hash does not match sha256(prev_hash + canonical entry) (row edited)");
            }
            prev = e.EntryHash!;
            i++;
        }
        return new PromotionVerifyResult(true, i, null, prev, Path, null);
    }

    private IEnumerable<string> ReadLinesLocked()
    {
        if (!File.Exists(Path)) { yield break; }
        using var fs = new FileStream(Path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite);
        using var reader = new StreamReader(fs, Encoding.UTF8);
        string? line;
        while ((line = reader.ReadLine()) is not null)
        {
            yield return line;
        }
    }

    private static PromotionEntry? TryParse(string line)
    {
        if (string.IsNullOrWhiteSpace(line)) { return null; }
        try
        {
            var e = JsonSerializer.Deserialize<PromotionEntry>(line, Canonical);
            if (e is null || string.IsNullOrEmpty(e.EntryHash) || string.IsNullOrEmpty(e.PrevHash) || e.Ci95 is null || e.Ci95.Length != 2) { return null; }
            return e;
        }
        catch (JsonException) { return null; }
    }
}
