using System;

namespace VedAstro.Library
{
    /// <summary>
    /// jyotish-os replacement for VedAstro's hidden <c>Secrets</c> partial class.
    /// Resolves ONLY from environment variables named <c>VEDASTRO_SECRET_&lt;KEY&gt;</c>
    /// (key upper-cased). Returns "" when absent — never throws, never reads a file,
    /// never contacts a network. No Azure / OpenAI / Cohere / Bing / Blob / Table code
    /// is compiled into this assembly, so an empty value can never enable a cloud path.
    /// </summary>
    public static partial class Secrets
    {
        public const string EnvPrefix = "VEDASTRO_SECRET_";

        public static string Get(string key)
        {
            if (string.IsNullOrWhiteSpace(key)) { return ""; }
            var envName = EnvPrefix + key.Trim().ToUpperInvariant();
            var value = Environment.GetEnvironmentVariable(envName);
            return value ?? "";
        }
    }
}
