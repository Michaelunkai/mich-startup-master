$ErrorActionPreference = 'Stop'

Add-Type @'
using System;
using System.Runtime.InteropServices;

[StructLayout(LayoutKind.Sequential, Pack = 4)]
public struct PropertyKey
{
    public Guid fmtid;
    public uint pid;
    public PropertyKey(Guid formatId, uint propertyId)
    {
        fmtid = formatId;
        pid = propertyId;
    }
}

[StructLayout(LayoutKind.Explicit)]
public struct PropVariant
{
    [FieldOffset(0)]
    public ushort vt;

    [FieldOffset(8)]
    public IntPtr pointerValue;

    public static PropVariant FromString(string value)
    {
        PropVariant result = new PropVariant();
        result.vt = 31;
        result.pointerValue = Marshal.StringToCoTaskMemUni(value);
        return result;
    }
}

[ComImport]
[InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
[Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99")]
public interface IPropertyStore
{
    void GetCount(out uint count);
    void GetAt(uint index, out PropertyKey key);
    void GetValue(ref PropertyKey key, out PropVariant value);
    void SetValue(ref PropertyKey key, ref PropVariant value);
    void Commit();
}

[ComImport]
[InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
[Guid("0000010B-0000-0000-C000-000000000046")]
public interface IPersistFile
{
    void GetClassID(out Guid classId);
    [PreserveSig] int IsDirty();
    void Load([MarshalAs(UnmanagedType.LPWStr)] string fileName, uint mode);
    void Save([MarshalAs(UnmanagedType.LPWStr)] string fileName, bool remember);
    void SaveCompleted([MarshalAs(UnmanagedType.LPWStr)] string fileName);
    void GetCurFile([MarshalAs(UnmanagedType.LPWStr)] out string fileName);
}

public static class ShortcutProperties
{
    private static readonly PropertyKey AppUserModelId = new PropertyKey(
        new Guid("9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3"), 5);

    [DllImport("ole32.dll")]
    private static extern int PropVariantClear(ref PropVariant value);

    public static void SetAppUserModelId(string shortcutPath, string appId)
    {
        object link = Activator.CreateInstance(Type.GetTypeFromCLSID(
            new Guid("00021401-0000-0000-C000-000000000046")));
        try
        {
            IPersistFile persistFile = (IPersistFile)link;
            persistFile.Load(shortcutPath, 2);
            IPropertyStore store = (IPropertyStore)link;
            PropertyKey key = AppUserModelId;
            PropVariant value = PropVariant.FromString(appId);
            try
            {
                store.SetValue(ref key, ref value);
                store.Commit();
                persistFile.Save(shortcutPath, true);
            }
            finally
            {
                PropVariantClear(ref value);
            }
        }
        finally
        {
            Marshal.ReleaseComObject(link);
        }
    }

    public static string GetAppUserModelId(string shortcutPath)
    {
        object link = Activator.CreateInstance(Type.GetTypeFromCLSID(
            new Guid("00021401-0000-0000-C000-000000000046")));
        try
        {
            IPersistFile persistFile = (IPersistFile)link;
            persistFile.Load(shortcutPath, 2);
            IPropertyStore store = (IPropertyStore)link;
            PropertyKey key = AppUserModelId;
            PropVariant value;
            store.GetValue(ref key, out value);
            try
            {
                return value.pointerValue == IntPtr.Zero ? "" : Marshal.PtrToStringUni(value.pointerValue);
            }
            finally
            {
                PropVariantClear(ref value);
            }
        }
        finally
        {
            Marshal.ReleaseComObject(link);
        }
    }
}
'@

$appId = 'Mich.MichStartupMaster'
$programsDirectory = Join-Path ([Environment]::GetFolderPath('StartMenu')) 'Programs'
$shortcutPath = Join-Path $programsDirectory 'Mich Startup Master.lnk'
$pinnedDirectory = Join-Path $env:APPDATA 'Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar'
$pinnedShortcutPath = Join-Path $pinnedDirectory 'Mich Startup Master.lnk'

if (-not (Test-Path -LiteralPath $shortcutPath -PathType Leaf)) {
    throw "Start Menu shortcut was not found: $shortcutPath"
}

[ShortcutProperties]::SetAppUserModelId($shortcutPath, $appId)

$shell = New-Object -ComObject Shell.Application
$programsFolder = $shell.Namespace($programsDirectory)
$programsItem = $programsFolder.ParseName((Split-Path -Leaf $shortcutPath))

if (Test-Path -LiteralPath $pinnedShortcutPath -PathType Leaf) {
    [ShortcutProperties]::SetAppUserModelId($pinnedShortcutPath, $appId)
    $unpinVerb = $programsItem.Verbs() |
        Where-Object { $_.Name.Replace('&', '').Trim() -match '(?i)unpin from taskbar' } |
        Select-Object -First 1
    if ($null -ne $unpinVerb) {
        $unpinVerb.DoIt()
        Start-Sleep -Seconds 2
    }
}

$programsFolder = $shell.Namespace($programsDirectory)
$programsItem = $programsFolder.ParseName((Split-Path -Leaf $shortcutPath))
$pinVerb = $programsItem.Verbs() |
    Where-Object { $_.Name.Replace('&', '').Trim() -match '(?i)pin to taskbar' } |
    Select-Object -First 1
if ($null -eq $pinVerb) {
    Copy-Item -LiteralPath $shortcutPath -Destination $pinnedShortcutPath -Force
    [ShortcutProperties]::SetAppUserModelId($pinnedShortcutPath, $appId)
    $pinResult = 'restored through the Windows pinned-items store'
} else {
    $pinVerb.DoIt()
    Start-Sleep -Seconds 2
    $pinResult = 'native pin command invoked'
}

if (-not (Test-Path -LiteralPath $pinnedShortcutPath -PathType Leaf)) {
    throw "Windows did not create the taskbar pin: $pinnedShortcutPath"
}
[ShortcutProperties]::SetAppUserModelId($pinnedShortcutPath, $appId)

[pscustomobject]@{
    StartMenuAppId = [ShortcutProperties]::GetAppUserModelId($shortcutPath)
    PinnedAppId = [ShortcutProperties]::GetAppUserModelId($pinnedShortcutPath)
    PinResult = $pinResult
    PinnedShortcut = $pinnedShortcutPath
} | Format-List
