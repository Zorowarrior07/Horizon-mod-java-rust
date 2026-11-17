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
 * <p>Two things changed in 1.21.3 and both of them bite at runtime, not at
 * compile time — this file exists because a server boot found them:
 *
 * <ol>
 *   <li>{@code AbstractBlock.Settings} must now be told the block's own registry
 *       key before the {@code Block} is constructed, or the constructor throws
 *       {@code NullPointerException: Block id not set} while working out the
 *       default loot table.</li>
 *   <li>Once a key is set, constructing the block creates an <i>intrusive</i>
 *       registry holder, and the frozen block registry refuses those after
 *       startup: {@code IllegalStateException: This registry can't create
 *       intrusive holders}. So unlike 1.21.1, the block has to be created while
 *       the registry is still open.</li>
 * </ol>
 *
 * <p>Consequence: on 1.21.3+ this mod needs the one hook that lets a mod add
 * blocks before the registry freezes. That is exactly what Fabric API provides,
 * so this variant depends on it (see build.gradle {@code fabric_api_version})
 * and registers inside {@code RegistryEvent}. The 1.21.1 variant still uses
 * nothing but vanilla classes.
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
        // Registration is deferred to the registry-open phase; see the class doc.
        net.fabricmc.fabric.api.event.registry.RegistryEvent.register(Registries.BLOCK)
                .register(id("bridge_beacon"), (key, registry) -> {
                    Block block = new Block(AbstractBlock.Settings.create()
                            .registryKey(blockKey("bridge_beacon"))
                            .strength(3.5F, 6.0F)
                            .luminance(state -> 12)
                            .sounds(BlockSoundGroup.STONE)
                            .requiresTool());
                    registry.register(key, block);
                    bridgeBeacon = block;
                });

        net.fabricmc.fabric.api.event.registry.RegistryEvent.register(Registries.ITEM)
                .register(id("bridge_chip"), (key, registry) -> {
                    bridgeChip = registry.register(key, (Item) new Item(new Item.Settings()));
                });

        net.fabricmc.fabric.api.event.registry.RegistryEvent.register(Registries.ITEM)
                .register(id("bridge_beacon"), (key, registry) -> {
                    registry.register(key, new BlockItem(bridgeBeacon, new Item.Settings()));
                });

        String version = FabricLoader.getInstance().getModContainer(MOD_ID)
                .map(c -> c.getMetadata().getVersion().getFriendlyString())
                .orElse("unknown");
        LOGGER.info("[ZoroHorizonBridge] mod {} loaded: bridge_beacon + bridge_chip registered",
                version);
    }
}
