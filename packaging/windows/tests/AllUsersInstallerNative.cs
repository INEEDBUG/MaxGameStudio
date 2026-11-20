using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Security.Principal;
using System.Runtime.InteropServices;
using System.Text;

public static class MgsAllUsersNative
{
    private delegate bool EnumCallback(IntPtr window, IntPtr data);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr LoadLibraryEx(string path, IntPtr file, uint flags);
    [DllImport("kernel32.dll", EntryPoint = "FindResourceW")]
    private static extern IntPtr FindResource(IntPtr module, IntPtr name, IntPtr type);
    [DllImport("kernel32.dll")]
    private static extern IntPtr LoadResource(IntPtr module, IntPtr resource);
    [DllImport("kernel32.dll")]
    private static extern uint SizeofResource(IntPtr module, IntPtr resource);
    [DllImport("kernel32.dll")]
    private static extern IntPtr LockResource(IntPtr data);
    [DllImport("kernel32.dll")]
    private static extern bool FreeLibrary(IntPtr module);
    [DllImport("kernel32.dll")]
    private static extern bool CloseHandle(IntPtr handle);
    [DllImport("advapi32.dll", SetLastError = true)]
    private static extern bool OpenProcessToken(IntPtr process, uint access, out IntPtr token);

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumCallback callback, IntPtr data);
    [DllImport("user32.dll")]
    private static extern bool EnumChildWindows(IntPtr parent, EnumCallback callback, IntPtr data);
    [DllImport("user32.dll")]
    private static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")]
    private static extern bool IsWindowEnabled(IntPtr window);
    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassName(IntPtr window, StringBuilder name, int capacity);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongW")]
    private static extern int GetWindowLong(IntPtr window, int index);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr SendMessageTimeout(IntPtr window, uint message, IntPtr parameter,
        StringBuilder text, uint flags, uint milliseconds, out IntPtr result);
    [DllImport("user32.dll", EntryPoint = "SendMessageTimeoutW")]
    private static extern IntPtr SendMessageTimeoutValue(IntPtr window, uint message, IntPtr parameter,
        IntPtr data, uint flags, uint milliseconds, out IntPtr result);
    [DllImport("user32.dll", EntryPoint = "SendMessageTimeoutW", CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern IntPtr SendMessageTimeoutText(IntPtr window, uint message, IntPtr parameter,
        string text, uint flags, uint milliseconds, out IntPtr result);
    [DllImport("user32.dll")]
    private static extern bool PostMessage(IntPtr window, uint message, IntPtr parameter, IntPtr data);

    public static bool IsStandardUser(IntPtr process, string expectedSid)
    {
        if (!OpenProcessToken(process, 0x000A, out IntPtr token))
            throw new Win32Exception(Marshal.GetLastWin32Error());
        try
        {
            using (var identity = new WindowsIdentity(token))
            {
                return identity.User != null && identity.User.Value == expectedSid &&
                    !new WindowsPrincipal(identity).IsInRole(WindowsBuiltInRole.Administrator);
            }
        }
        finally { CloseHandle(token); }
    }

    public static string ReadManifest(string installer)
    {
        // Map the executable as data; do not execute or load its dependencies.
        IntPtr module = LoadLibraryEx(installer, IntPtr.Zero, 0x22);
        if (module == IntPtr.Zero) throw new InvalidOperationException("Cannot map installer resources.");
        try
        {
            IntPtr resource = FindResource(module, new IntPtr(1), new IntPtr(24));
            if (resource == IntPtr.Zero) throw new InvalidOperationException("Installer manifest is missing.");
            int size = checked((int)SizeofResource(module, resource));
            IntPtr bytes = LockResource(LoadResource(module, resource));
            if (size == 0 || bytes == IntPtr.Zero) throw new InvalidOperationException("Cannot read installer manifest.");
            byte[] result = new byte[size];
            Marshal.Copy(bytes, result, 0, size);
            return Encoding.UTF8.GetString(result).Trim('\uFEFF', '\0');
        }
        finally { FreeLibrary(module); }
    }

    public sealed class Control
    {
        public IntPtr Handle { get; set; }
        public IntPtr Dialog { get; set; }
        public string Text { get; set; }
        public bool IsButton { get; set; }
        public bool IsEdit { get; set; }
        public bool IsCheckBox { get; set; }
        public bool Enabled { get; set; }
        public bool IsRadioButton { get; set; }
    }

    public static Control[] VisibleControls(uint expectedPid)
    {
        var controls = new List<Control>();
        EnumWindows((window, data) =>
        {
            GetWindowThreadProcessId(window, out uint pid);
            if (pid != expectedPid || !IsWindowVisible(window)) return true;
            EnumChildWindows(window, (child, unused) =>
            {
                if (!IsWindowVisible(child)) return true;
                var className = new StringBuilder(64);
                GetClassName(child, className, className.Capacity);
                string kind = className.ToString();
                bool isButton = kind == "Button";
                bool isEdit = kind == "Edit";
                var text = new StringBuilder(2048);
                if (SendMessageTimeout(child, 0x000D, new IntPtr(text.Capacity), text, 2, 250, out _) == IntPtr.Zero
                    || (text.Length == 0 && !isEdit)) return true;
                int buttonStyle = GetWindowLong(child, -16) & 0x0f;
                controls.Add(new Control {
                    Handle = child, Dialog = window, Text = text.ToString(),
                    IsButton = isButton, IsEdit = isEdit,
                    Enabled = IsWindowEnabled(child),
                    IsCheckBox = isButton && (buttonStyle == 2 || buttonStyle == 3 || buttonStyle == 5 || buttonStyle == 6),
                    IsRadioButton = isButton && (buttonStyle == 4 || buttonStyle == 9)
                });
                return true;
            }, IntPtr.Zero);
            return true;
        }, IntPtr.Zero);
        return controls.ToArray();
    }

    public static bool Click(IntPtr button) => PostMessage(button, 0x00F5, IntPtr.Zero, IntPtr.Zero);

    public static bool SetEditText(IntPtr edit, string text) =>
        SendMessageTimeoutText(edit, 0x000C, IntPtr.Zero, text, 2, 500, out IntPtr result) != IntPtr.Zero
        && result != IntPtr.Zero;

    public static bool ClearCheckBox(IntPtr button) =>
        SendMessageTimeoutValue(button, 0x00F1, IntPtr.Zero, IntPtr.Zero, 2, 500, out _) != IntPtr.Zero
        && CheckedState(button) == 0;

    public static int CheckedState(IntPtr button)
    {
        IntPtr success = SendMessageTimeoutValue(button, 0x00F0, IntPtr.Zero, IntPtr.Zero, 2, 500, out IntPtr state);
        if (success == IntPtr.Zero) throw new InvalidOperationException("Could not read the installer button state.");
        return state.ToInt32();
    }
}
