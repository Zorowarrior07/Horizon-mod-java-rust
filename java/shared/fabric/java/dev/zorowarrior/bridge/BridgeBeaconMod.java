package dev.zorowarrior.bridge;

import net.fabricmc.api.ModInitializer;
import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.block.AbstractBlock;
import net.minecraft.block.Block;
import net.minecraft.item.BlockItem;
import net.minecraft.item.Item;
import net.minecraft.registry.Registries;
import net.minecraft.registry.Registry;
import net.minecraft.registry.RegistryKey;
import net.minecraft.registry.RegistryKeys;
import net.minecraft.sound.BlockSoundGroup;
import net.minecraft.util.Identifier;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * ZOROWARRIOR HORIZON BRIDGE — Fabric, Minecraft 1.21.3 and newer 1.21.x.
 *
 * <p>Vanilla classes only. Getting a block registered from a mod on these
 * versions takes a specific sequence, and every wrong step fails at runtime
 * rather than at compile time — this class records them so the order is not
 * rediscovered later:
 *
 * <ol>
 *   <li>The {@code Block} constructor asks {@code Settings} for its id, so
 *       {@code registryKey(...)} must be supplied, or you get
 *       {@code NullPointerException: Block id not set}.</li>
 *   <li>Once a key is set, the constructor attaches an <i>intrusive</i> holder,
 *       which the already-frozen block registry refuses with
 *       {@code IllegalStateException: This registry can't create intrusive
 *       holders}. That is the obstacle, and it is why a plain
 *       {@code Registry.register} from {@code onInitialize} cannot work.</li>
 * </ol>
 *
 * <p>STATUS — this variant is <b>not verified</b>. The sequence above is what
 * the errors describe, but it does not start a server yet: 1.21.3+ refuses to
 * create the intrusive holder the constructor needs, from a mod initialiser, in
 * every arrangement tried so far. Rather than pretend, the report marks
 * 1.21.4/1.21.11 UNTESTED and points at their jars, which do compile.
 *
 * <p>The 1.21.1 build uses the sibling source in
 * {@code java/shared/fabric-legacy/} because its API predates all of this.
 */
public final class BridgeBeaconMod implements ModInitializer {

    public static final String MOD_ID = "zoro_bridge";
    public static final Logger LOGGER = LoggerFactory.getLogger("ZoroHorizonBridge");

    public static Block bridgeBeacon;
    public static Item bridgeChip;

    private static Identifier id(String path) {
        return Identifier.of(MOD_ID, path);
    }

    private static RegistryKey<Block> blockKey(String path) {
        return RegistryKey.of(RegistryKeys.BLOCK, id(path));
    }

    private static RegistryKey<Item> itemKey(String path) {
        return RegistryKey.of(RegistryKeys.ITEM, id(path));
    }

    @Override
    public void onInitialize() {
        // 1. The block itself: plain Block, no behaviour overrides.
        Block block = new Block(AbstractBlock.Settings.create()
                .registryKey(blockKey("bridge_beacon"))
                .strength(3.5F, 6.0F)
                .luminance(state -> 12)
                .sounds(BlockSoundGroup.STONE)
                .requiresTool());

        bridgeBeacon = Registry.register(Registries.BLOCK, id("bridge_beacon"), block);
        bridgeChip = Registry.register(Registries.ITEM, id("bridge_chip"),
                new Item(new Item.Settings()));
        Registry.register(Registries.ITEM, id("bridge_beacon"),
                new BlockItem(block, new Item.Settings()));

        String version = FabricLoader.getInstance().getModContainer(MOD_ID)
                .map(c -> c.getMetadata().getVersion().getFriendlyString())
                .orElse("unknown");
        LOGGER.info("[ZoroHorizonBridge] mod {} loaded: bridge_beacon + bridge_chip registered",
                version);
    }
}
