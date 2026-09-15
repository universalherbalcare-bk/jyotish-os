using System;
using System.Threading.Tasks;

namespace VedAstro.Library
{
    /// <summary>
    /// Local-only logger (jyotish-os). Upstream LibLogger posted to an Azure endpoint via HttpClient;
    /// this one writes to stderr and nothing else.
    /// </summary>
    public static class LibLogger
    {
        public static Task Debug(string message = "") { Console.Error.WriteLine("[vedastro.debug] " + message); return Task.CompletedTask; }
        public static void Debug(Exception e, string s) { Console.Error.WriteLine("[vedastro.debug] " + s + " :: " + e); }
        public static void Debug(Exception e) { Console.Error.WriteLine("[vedastro.debug] " + e); }
        public static Task Error(Exception e, string extraInfo = "") { Console.Error.WriteLine("[vedastro.error] " + extraInfo + " :: " + e); return Task.CompletedTask; }
        public static Task Error(string message) { Console.Error.WriteLine("[vedastro.error] " + message); return Task.CompletedTask; }
    }

    /// <summary>Local-only stand-in for upstream LogManager (which wrote to an XML log file / Azure).</summary>
    public static class LogManager
    {
        public static void Error(Exception e) { Console.Error.WriteLine("[vedastro.error] " + e); }
        public static void Error(string errorMessage) { Console.Error.WriteLine("[vedastro.error] " + errorMessage); }
        public static void Debug(string message) { Console.Error.WriteLine("[vedastro.debug] " + message); }
    }
}
