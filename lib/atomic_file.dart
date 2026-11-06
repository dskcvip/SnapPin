import 'dart:io';

/// Stage on the destination volume, then replace in one rename. Never delete
/// the user's existing file before the new bytes have been written successfully.
Future<void> atomicWrite(File target, List<int> bytes) async {
  await target.parent.create(recursive: true);
  final staging = await target.parent.createTemp('.snappin-write-');
  final temporary = File('${staging.path}/data');
  try {
    await temporary.writeAsBytes(bytes, flush: true);
    await temporary.rename(target.path);
  } finally {
    if (await temporary.exists()) await temporary.delete();
    await staging.delete();
  }
}
