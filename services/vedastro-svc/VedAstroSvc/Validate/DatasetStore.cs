using System.Globalization;
using System.Security.Cryptography;
using System.Text.Json;
using Microsoft.Data.Sqlite;

namespace VedAstroSvc.Validate;

/// <summary>One person row ready for VedAstro: birth as std time string ("HH:mm dd/MM/yyyy zzz") + lat/lon.</summary>
public sealed record PersonRow(string Id, string StdTime, double Lat, double Lon, string LocationName);

/// <summary>
/// Loads VedAstro's HuggingFace CSVs into an embedded SQLite file on first use (idempotent: keyed by
/// the CSV SHA-256, rebuilt only when the CSV changes). Two logical datasets:
///   person   = PersonList-15k.csv        outcome columns: male, female, rodden_aa
///   marriage = PersonList-15k.csv JOIN MarriageInfoDataset.csv (by RowKey)
///              outcome columns: married, marriage_count, multiple_marriages, any_dissolution,
///              dissolution_count, any_happiness, all_happiness, any_struggle_or_tragedy,
///              love_count, arranged_count, any_arranged
/// "Truthy" = numeric value &gt; 0.
/// </summary>
public sealed class DatasetStore : IDisposable
{
    public static readonly IReadOnlyDictionary<string, IReadOnlyList<string>> OutcomeColumns = new Dictionary<string, IReadOnlyList<string>>
    {
        ["person"] = new[] { "male", "female", "rodden_aa" },
        ["marriage"] = new[] { "married", "marriage_count", "multiple_marriages", "any_dissolution", "dissolution_count", "any_happiness", "all_happiness", "any_struggle_or_tragedy", "love_count", "arranged_count", "any_arranged" },
    };

    private readonly string _csvDir;
    private readonly string _dbPath;
    private readonly object _gate = new();
    private SqliteConnection? _conn;
    private bool _loaded;

    public string PersonsSha256 { get; private set; } = "";
    public string MarriageSha256 { get; private set; } = "";
    public int PersonRows { get; private set; }
    public int MarriageRows { get; private set; }
    public string DbPath => _dbPath;
    public bool IsLoaded => _loaded;

    public DatasetStore(string csvDir, string dataDir)
    {
        _csvDir = csvDir;
        Directory.CreateDirectory(dataDir);
        _dbPath = Path.Combine(dataDir, "vedastro-datasets.sqlite");
    }

    public void EnsureLoaded()
    {
        lock (_gate)
        {
            if (_loaded) { return; }
            var personsCsv = Path.Combine(_csvDir, "PersonList-15k.csv");
            var marriageCsv = Path.Combine(_csvDir, "MarriageInfoDataset.csv");
            if (!File.Exists(personsCsv)) { throw new FileNotFoundException("dataset missing", personsCsv); }
            if (!File.Exists(marriageCsv)) { throw new FileNotFoundException("dataset missing", marriageCsv); }
            PersonsSha256 = Sha256File(personsCsv);
            MarriageSha256 = Sha256File(marriageCsv);

            _conn = new SqliteConnection(new SqliteConnectionStringBuilder { DataSource = _dbPath, Mode = SqliteOpenMode.ReadWriteCreate, Cache = SqliteCacheMode.Shared }.ToString());
            _conn.Open();
            Exec("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;");
            Exec("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)");
            var stamp = $"{PersonsSha256}:{MarriageSha256}:v1";
            if (ReadMeta("stamp") != stamp)
            {
                Rebuild(personsCsv, marriageCsv, stamp);
            }
            PersonRows = ScalarInt("SELECT COUNT(*) FROM persons");
            MarriageRows = ScalarInt("SELECT COUNT(*) FROM marriage");
            _loaded = true;
        }
    }

    public bool HasDataset(string name) => OutcomeColumns.ContainsKey(name);
    public bool HasOutcome(string dataset, string column) => OutcomeColumns.TryGetValue(dataset, out var cols) && cols.Contains(column, StringComparer.Ordinal);

    /// <summary>Rows in deterministic id order, with the outcome column value (numeric).</summary>
    public List<(PersonRow Person, double Outcome)> Rows(string dataset, string outcomeColumn, int offset, int limit)
    {
        EnsureLoaded();
        if (!HasOutcome(dataset, outcomeColumn)) { throw new ArgumentException($"unknown outcome column '{outcomeColumn}' for dataset '{dataset}'"); }
        var sql = dataset == "person"
            ? $"SELECT p.id, p.std_time, p.lat, p.lon, p.location, p.{outcomeColumn} FROM persons p WHERE p.parse_ok=1 ORDER BY p.id LIMIT @limit OFFSET @offset"
            : $"SELECT p.id, p.std_time, p.lat, p.lon, p.location, m.{outcomeColumn} FROM persons p JOIN marriage m ON m.person_id=p.id WHERE p.parse_ok=1 ORDER BY p.id LIMIT @limit OFFSET @offset";
        var list = new List<(PersonRow, double)>();
        using var cmd = _conn!.CreateCommand();
        cmd.CommandText = sql;
        cmd.Parameters.AddWithValue("@limit", limit);
        cmd.Parameters.AddWithValue("@offset", offset);
        using var r = cmd.ExecuteReader();
        while (r.Read())
        {
            list.Add((new PersonRow(r.GetString(0), r.GetString(1), r.GetDouble(2), r.GetDouble(3), r.GetString(4)), r.GetDouble(5)));
        }
        return list;
    }

    /// <summary>Fraction of truthy outcome over the whole dataset (not just the sampled rows).</summary>
    public double BaseRateFull(string dataset, string outcomeColumn)
    {
        EnsureLoaded();
        if (!HasOutcome(dataset, outcomeColumn)) { throw new ArgumentException($"unknown outcome column '{outcomeColumn}' for dataset '{dataset}'"); }
        var sql = dataset == "person"
            ? $"SELECT AVG(CASE WHEN {outcomeColumn}>0 THEN 1.0 ELSE 0.0 END) FROM persons WHERE parse_ok=1"
            : $"SELECT AVG(CASE WHEN m.{outcomeColumn}>0 THEN 1.0 ELSE 0.0 END) FROM persons p JOIN marriage m ON m.person_id=p.id WHERE p.parse_ok=1";
        using var cmd = _conn!.CreateCommand();
        cmd.CommandText = sql;
        var v = cmd.ExecuteScalar();
        return v is DBNull or null ? 0 : Convert.ToDouble(v, CultureInfo.InvariantCulture);
    }

    private void Rebuild(string personsCsv, string marriageCsv, string stamp)
    {
        Exec("DROP TABLE IF EXISTS persons; DROP TABLE IF EXISTS marriage;");
        Exec(@"CREATE TABLE persons (
                id TEXT PRIMARY KEY, name TEXT, gender TEXT, std_time TEXT, lat REAL, lon REAL, location TEXT,
                parse_ok INTEGER NOT NULL, male INTEGER NOT NULL, female INTEGER NOT NULL, rodden_aa INTEGER NOT NULL)");
        Exec(@"CREATE TABLE marriage (
                person_id TEXT PRIMARY KEY, married INTEGER NOT NULL, marriage_count INTEGER NOT NULL, multiple_marriages INTEGER NOT NULL,
                any_dissolution INTEGER NOT NULL, dissolution_count INTEGER NOT NULL, any_happiness INTEGER NOT NULL, all_happiness INTEGER NOT NULL,
                any_struggle_or_tragedy INTEGER NOT NULL, love_count INTEGER NOT NULL, arranged_count INTEGER NOT NULL, any_arranged INTEGER NOT NULL)");

        using (var tx = _conn!.BeginTransaction())
        {
            using var ins = _conn.CreateCommand();
            ins.Transaction = tx;
            ins.CommandText = @"INSERT OR REPLACE INTO persons (id,name,gender,std_time,lat,lon,location,parse_ok,male,female,rodden_aa)
                                VALUES (@id,@name,@gender,@std,@lat,@lon,@loc,@ok,@male,@female,@aa)";
            var p = new Dictionary<string, SqliteParameter>();
            foreach (var n in new[] { "@id", "@name", "@gender", "@std", "@lat", "@lon", "@loc", "@ok", "@male", "@female", "@aa" }) { p[n] = ins.Parameters.Add(n, SqliteType.Text); }
            p["@lat"].SqliteType = SqliteType.Real; p["@lon"].SqliteType = SqliteType.Real;
            foreach (var n in new[] { "@ok", "@male", "@female", "@aa" }) { p[n].SqliteType = SqliteType.Integer; }

            foreach (var rec in Csv.Read(personsCsv))
            {
                var id = rec.GetValueOrDefault("RowKey") ?? "";
                if (id.Length == 0) { continue; }
                var gender = rec.GetValueOrDefault("Gender") ?? "";
                var notes = rec.GetValueOrDefault("Notes") ?? "";
                var ok = TryParseBirth(rec.GetValueOrDefault("BirthTime") ?? "", out var std, out var lat, out var lon, out var loc);
                p["@id"].Value = id; p["@name"].Value = rec.GetValueOrDefault("Name") ?? ""; p["@gender"].Value = gender;
                p["@std"].Value = std; p["@lat"].Value = lat; p["@lon"].Value = lon; p["@loc"].Value = loc;
                p["@ok"].Value = ok ? 1 : 0;
                p["@male"].Value = gender.Equals("Male", StringComparison.OrdinalIgnoreCase) ? 1 : 0;
                p["@female"].Value = gender.Equals("Female", StringComparison.OrdinalIgnoreCase) ? 1 : 0;
                p["@aa"].Value = notes.Contains("'AA'", StringComparison.Ordinal) ? 1 : 0;
                ins.ExecuteNonQuery();
            }
            tx.Commit();
        }

        using (var tx = _conn!.BeginTransaction())
        {
            using var ins = _conn.CreateCommand();
            ins.Transaction = tx;
            ins.CommandText = @"INSERT OR REPLACE INTO marriage (person_id,married,marriage_count,multiple_marriages,any_dissolution,dissolution_count,any_happiness,all_happiness,any_struggle_or_tragedy,love_count,arranged_count,any_arranged)
                                VALUES (@pid,@married,@mc,@mm,@ad,@dc,@ah,@allh,@ast,@lc,@ac,@aa)";
            var names = new[] { "@pid", "@married", "@mc", "@mm", "@ad", "@dc", "@ah", "@allh", "@ast", "@lc", "@ac", "@aa" };
            var p = names.ToDictionary(n => n, n => ins.Parameters.Add(n, n == "@pid" ? SqliteType.Text : SqliteType.Integer));
            foreach (var rec in Csv.Read(marriageCsv))
            {
                var id = rec.GetValueOrDefault("PartitionKey") ?? "";
                if (id.Length == 0) { continue; }
                var m = ParseMarriage(rec.GetValueOrDefault("Info") ?? "");
                p["@pid"].Value = id; p["@married"].Value = m.Count > 0 ? 1 : 0; p["@mc"].Value = m.Count; p["@mm"].Value = m.Count >= 2 ? 1 : 0;
                p["@ad"].Value = m.Dissolution > 0 ? 1 : 0; p["@dc"].Value = m.Dissolution; p["@ah"].Value = m.Happiness > 0 ? 1 : 0;
                p["@allh"].Value = (m.Count > 0 && m.Happiness == m.Count) ? 1 : 0; p["@ast"].Value = m.StruggleOrTragedy > 0 ? 1 : 0;
                p["@lc"].Value = m.Love; p["@ac"].Value = m.Arranged; p["@aa"].Value = m.Arranged > 0 ? 1 : 0;
                ins.ExecuteNonQuery();
            }
            tx.Commit();
        }
        Exec("INSERT OR REPLACE INTO meta(key,value) VALUES('stamp', @v)", ("@v", stamp));
    }

    private sealed record MarriageAgg(int Count, int Dissolution, int Happiness, int StruggleOrTragedy, int Love, int Arranged);

    private static MarriageAgg ParseMarriage(string json)
    {
        try
        {
            using var doc = JsonDocument.Parse(json);
            if (!doc.RootElement.TryGetProperty("marriages", out var arr) || arr.ValueKind != JsonValueKind.Array) { return new MarriageAgg(0, 0, 0, 0, 0, 0); }
            int count = 0, dis = 0, hap = 0, st = 0, love = 0, arr_ = 0;
            foreach (var m in arr.EnumerateArray())
            {
                count++;
                var outcome = (m.TryGetProperty("outcome", out var o) && o.ValueKind == JsonValueKind.String ? o.GetString() : "") ?? "";
                var type = (m.TryGetProperty("type", out var t) && t.ValueKind == JsonValueKind.String ? t.GetString() : "") ?? "";
                if (outcome.Equals("Dissolution", StringComparison.OrdinalIgnoreCase)) { dis++; }
                if (outcome.Equals("Happiness", StringComparison.OrdinalIgnoreCase)) { hap++; }
                if (outcome.StartsWith("Struggle", StringComparison.OrdinalIgnoreCase) || outcome.StartsWith("Tragic", StringComparison.OrdinalIgnoreCase) || outcome.Equals("Tragedy", StringComparison.OrdinalIgnoreCase) || outcome.Equals("Murder", StringComparison.OrdinalIgnoreCase)) { st++; }
                if (type.Equals("Love", StringComparison.OrdinalIgnoreCase)) { love++; }
                if (type.Equals("Arranged", StringComparison.OrdinalIgnoreCase)) { arr_++; }
            }
            return new MarriageAgg(count, dis, hap, st, love, arr_);
        }
        catch (JsonException)
        {
            return new MarriageAgg(0, 0, 0, 0, 0, 0);
        }
    }

    private static bool TryParseBirth(string json, out string stdTime, out double lat, out double lon, out string location)
    {
        stdTime = ""; lat = 0; lon = 0; location = "";
        try
        {
            using var doc = JsonDocument.Parse(json);
            var root = doc.RootElement;
            stdTime = root.TryGetProperty("StdTime", out var s) && s.ValueKind == JsonValueKind.String ? (s.GetString() ?? "") : "";
            if (!root.TryGetProperty("Location", out var l)) { return false; }
            location = l.TryGetProperty("Name", out var n) && n.ValueKind == JsonValueKind.String ? (n.GetString() ?? "") : "";
            if (!l.TryGetProperty("Latitude", out var la) || !l.TryGetProperty("Longitude", out var lo)) { return false; }
            if (la.ValueKind != JsonValueKind.Number || lo.ValueKind != JsonValueKind.Number) { return false; }
            lat = la.GetDouble(); lon = lo.GetDouble();
            if (stdTime.Length == 0 || lat < -90 || lat > 90 || lon < -180 || lon > 180) { return false; }
            // VedAstro std time format "HH:mm dd/MM/yyyy zzz"
            return DateTimeOffset.TryParseExact(stdTime, "HH:mm dd/MM/yyyy zzz", CultureInfo.InvariantCulture, DateTimeStyles.None, out _);
        }
        catch (JsonException) { return false; }
    }

    private void Exec(string sql, params (string, object)[] args)
    {
        using var cmd = _conn!.CreateCommand();
        cmd.CommandText = sql;
        foreach (var (k, v) in args) { cmd.Parameters.AddWithValue(k, v); }
        cmd.ExecuteNonQuery();
    }

    private string? ReadMeta(string key)
    {
        using var cmd = _conn!.CreateCommand();
        cmd.CommandText = "SELECT value FROM meta WHERE key=@k";
        cmd.Parameters.AddWithValue("@k", key);
        return cmd.ExecuteScalar() as string;
    }

    private int ScalarInt(string sql)
    {
        using var cmd = _conn!.CreateCommand();
        cmd.CommandText = sql;
        return Convert.ToInt32(cmd.ExecuteScalar(), CultureInfo.InvariantCulture);
    }

    private static string Sha256File(string path)
    {
        using var fs = File.OpenRead(path);
        return Convert.ToHexStringLower(SHA256.HashData(fs));
    }

    public void Dispose() { _conn?.Dispose(); }
}

/// <summary>Minimal RFC-4180 CSV reader (quoted fields with embedded newlines/commas/doubled quotes).</summary>
internal static class Csv
{
    public static IEnumerable<Dictionary<string, string>> Read(string path)
    {
        using var reader = new StreamReader(path, System.Text.Encoding.UTF8, detectEncodingFromByteOrderMarks: true);
        var header = ReadRecord(reader);
        if (header is null) { yield break; }
        while (true)
        {
            var rec = ReadRecord(reader);
            if (rec is null) { yield break; }
            if (rec.Count == 1 && rec[0].Length == 0) { continue; }
            var d = new Dictionary<string, string>(StringComparer.Ordinal);
            for (var i = 0; i < header.Count && i < rec.Count; i++) { d[header[i]] = rec[i]; }
            yield return d;
        }
    }

    private static List<string>? ReadRecord(StreamReader r)
    {
        if (r.Peek() < 0) { return null; }
        var fields = new List<string>();
        var sb = new System.Text.StringBuilder();
        var inQuotes = false;
        while (true)
        {
            var ci = r.Read();
            if (ci < 0)
            {
                fields.Add(sb.ToString());
                return fields;
            }
            var c = (char)ci;
            if (inQuotes)
            {
                if (c == '"')
                {
                    if (r.Peek() == '"') { r.Read(); sb.Append('"'); }
                    else { inQuotes = false; }
                }
                else { sb.Append(c); }
                continue;
            }
            switch (c)
            {
                case '"': inQuotes = true; break;
                case ',': fields.Add(sb.ToString()); sb.Clear(); break;
                case '\r': if (r.Peek() == '\n') { r.Read(); } fields.Add(sb.ToString()); return fields;
                case '\n': fields.Add(sb.ToString()); return fields;
                default: sb.Append(c); break;
            }
        }
    }
}
