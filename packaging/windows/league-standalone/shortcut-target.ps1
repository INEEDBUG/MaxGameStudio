function Get-MgsShortcutTarget([string]$LiteralPath) {
  if ($LiteralPath -notmatch '^[A-Za-z]:\\') { return '' }
  if (!('MgsShortcutTarget' -as [type])) {
    Add-Type -ErrorAction Stop -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class MgsShortcutTarget {
  [ComImport, Guid("00021401-0000-0000-C000-000000000046")]
  private class ShellLink {}

  [ComImport, Guid("0000010B-0000-0000-C000-000000000046"),
   InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  private interface IPersistFile {
    [PreserveSig] int GetClassID(out Guid classId);
    [PreserveSig] int IsDirty();
    [PreserveSig] int Load([MarshalAs(UnmanagedType.LPWStr)] string path, uint mode);
  }

  [ComImport, Guid("000214F9-0000-0000-C000-000000000046"),
   InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  private interface IShellLinkW {
    [PreserveSig]
    int GetPath([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder target,
                int capacity, IntPtr findData, uint flags);
  }

  public static string Read(string path) {
    const uint STGM_READ = 0;
    const uint SLGP_RAWPATH = 4;
    var link = new ShellLink();
    try {
      if (((IPersistFile)link).Load(path, STGM_READ) != 0) return string.Empty;
      var target = new StringBuilder(260);
      if (((IShellLinkW)link).GetPath(target, target.Capacity, IntPtr.Zero, SLGP_RAWPATH) != 0 ||
          target.Length >= target.Capacity - 1) return string.Empty;
      return target.ToString();
    } finally {
      Marshal.FinalReleaseComObject(link);
    }
  }
}
'@
  }
  [MgsShortcutTarget]::Read($LiteralPath)
}
